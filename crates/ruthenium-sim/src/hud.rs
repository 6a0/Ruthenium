//! What the HUD shows of the local player (`client/gui/Hud.java`), as the game thread keeps
//! and publishes it: the counters the HUD's own ticks keep (`Hud.tick`) and each frame's
//! record of the player's bars, effects and hotbar. Laying the record out and drawing it is
//! done elsewhere, on what a frame carries.

use ruthenium_data::v26_3::registries::DataComponentType as Kind;
use ruthenium_protocol::item::Component;

use crate::effect::Effect;
use crate::hud_text::HudTexts;
use crate::item::Stack;
use crate::player::LocalPlayer;

/// The hotbar's slots and the off hand after them.
pub const SLOTS: usize = 10;
/// The off hand's place in a record's slots.
pub const OFF_HAND: usize = 9;

/// How long the selected item's name shows: 40 ticks times the notification time
/// option, which is 1.
const HIGHLIGHT_TICKS: i32 = 40;
/// `ItemStack.setPopTime(5)`: how long a stack that grew in the hotbar bulges.
const POP_TICKS: i32 = 5;
/// `Entity.getMaxAirSupply` and `getTicksRequiredToFreeze`.
const MAX_AIR: i32 = 300;
const TICKS_TO_FREEZE: i32 = 140;

/// `ClientboundUpdateMobEffectPacket`'s flags.
const EFFECT_AMBIENT: u8 = 1;
const EFFECT_SHOW_ICON: u8 = 4;

/// The name a stack shows (`getHoverName`), as far as two stacks of one item can
/// differ in it: its own name and its item's name where the stack says them.
fn names(stack: &Stack) -> [Option<&Component>; 2] {
    [Kind::CustomName, Kind::ItemName].map(|kind| {
        stack
            .patch
            .added
            .iter()
            .find(|(k, _)| u32::try_from(*k).ok() == Some(kind.id()))
            .map(|(_, c)| c)
    })
}

/// The HUD's own state (`Hud`'s fields that its ticks change). It outlives levels.
#[derive(Debug, Clone, Default)]
pub struct HudState {
    /// `tickCount`.
    ticks: i32,
    /// `toolHighlightTimer` and `lastToolHighlight`.
    highlight_timer: i32,
    last_highlight: Stack,
    /// `ItemStack.popTime` of the stacks in the hotbar, by slot.
    pop: [i32; SLOTS],
}

impl HudState {
    /// `Hud.tick`, and the hotbar's part of `Inventory.tick`. Returns whether the
    /// selected stack is another, or has changed ([`Self::selected`]).
    pub fn tick(&mut self, player: &LocalPlayer) -> bool {
        self.ticks = self.ticks.wrapping_add(1);
        let selected = player.inventory.hand(crate::inventory::Hand::Main);
        if selected.is_empty() {
            self.highlight_timer = 0;
        } else if self.last_highlight.is_empty()
            || !selected.same_item(&self.last_highlight)
            || names(selected) != names(&self.last_highlight)
        {
            self.highlight_timer = HIGHLIGHT_TICKS;
        } else if self.highlight_timer > 0 {
            self.highlight_timer -= 1;
        }
        let changed = *selected != self.last_highlight;
        if changed {
            self.last_highlight = selected.clone();
        }
        for pop in &mut self.pop {
            *pop = (*pop - 1).max(0);
        }
        changed
    }

    /// The stack whose name shows, as the last tick found it.
    pub fn selected(&self) -> &Stack {
        &self.last_highlight
    }

    /// How long its name still shows, in ticks.
    pub fn highlight(&self) -> i32 {
        self.highlight_timer
    }

    /// `handleContainerSetSlot`: the server put `new` where `old` was, in the hotbar's
    /// slot `slot`. A stack that came or grew bulges; any other is a new one, at rest.
    pub fn slot_set(&mut self, slot: usize, old: &Stack, new: &Stack) {
        if let Some(pop) = self.pop.get_mut(slot) {
            let grew = !new.is_empty() && (old.is_empty() || old.count < new.count);
            *pop = if grew { POP_TICKS } else { 0 };
        }
    }

    /// A stack was put into a slot some other way (a new object, at rest).
    pub fn slot_replaced(&mut self, slot: usize) {
        if let Some(pop) = self.pop.get_mut(slot) {
            *pop = 0;
        }
    }

    /// The whole inventory was replaced.
    pub fn slots_replaced(&mut self) {
        self.pop = [0; SLOTS];
    }

    pub fn ticks(&self) -> i32 {
        self.ticks
    }
}

/// An effect as the HUD shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudEffect {
    /// The `minecraft:mob_effect` registry id.
    pub id: i32,
    pub amplifier: i32,
    /// Ticks left, or -1 for one that does not end.
    pub duration: i32,
    pub ambient: bool,
    pub show_icon: bool,
}

impl From<&Effect> for HudEffect {
    fn from(e: &Effect) -> HudEffect {
        HudEffect {
            id: e.id,
            amplifier: e.amplifier,
            duration: e.duration,
            ambient: e.flags & EFFECT_AMBIENT != 0,
            show_icon: e.flags & EFFECT_SHOW_ICON != 0,
        }
    }
}

/// A cooldown as of the frame's last tick (`ItemCooldowns`): the ticks it has left and
/// the ticks it lasts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cooldown {
    pub left: f32,
    pub of: f32,
}

impl Cooldown {
    /// `getCooldownPercent`, `partial` of a tick on.
    pub fn percent(&self, partial: f32) -> f32 {
        if self.of == 0.0 && self.left == 0.0 {
            return 0.0;
        }
        let v = (self.left - partial) / self.of;
        if v < 0.0 {
            0.0
        } else if v.is_nan() {
            v
        } else {
            v.min(1.0)
        }
    }
}

/// The local player as the HUD shows it, as its last tick left it.
#[derive(Debug, Clone, PartialEq)]
pub struct HudRecord {
    /// `Hud.tickCount`: the HUD's ticks so far.
    pub ticks: i32,
    /// `canHurtPlayer` and `hasExperience`: survival or adventure mode.
    pub survival: bool,
    /// The local game mode is spectator.
    pub spectator: bool,
    pub hardcore: bool,
    pub health: f32,
    /// The `max_health` attribute's value.
    pub max_health: f32,
    pub absorption: f32,
    /// `damageCooldownTime`: ticks since health was last set lower (20) or higher (10).
    pub damage_cooldown: i32,
    /// `getArmorValue`.
    pub armor: i32,
    pub food: i32,
    pub saturation: f32,
    pub air: i32,
    pub max_air: i32,
    pub eye_in_water: bool,
    /// `isFullyFrozen` and `getPercentFrozen`.
    pub fully_frozen: bool,
    pub frozen: f32,
    pub experience_progress: f32,
    pub experience_level: i32,
    pub effects: Vec<HudEffect>,
    pub selected: i32,
    /// The hotbar's stacks and the off hand's ([`OFF_HAND`]).
    pub slots: [Stack; SLOTS],
    /// How long each still bulges, in ticks.
    pub pop: [i32; SLOTS],
    pub cooldowns: [Cooldown; SLOTS],
    /// The main hand is the left one.
    pub left_handed: bool,
    /// `getAttackStrengthScale(0)` and `getCurrentItemAttackStrengthDelay`.
    pub attack_strength: f32,
    pub attack_delay: f32,
    /// The crosshair is on something alive that an attack would reach.
    pub target_alive: bool,
    pub texts: HudTexts,
}

impl Default for HudRecord {
    fn default() -> HudRecord {
        HudRecord {
            ticks: 0,
            survival: false,
            spectator: false,
            hardcore: false,
            health: 0.0,
            max_health: 20.0,
            absorption: 0.0,
            damage_cooldown: 0,
            armor: 0,
            food: 0,
            saturation: 0.0,
            air: MAX_AIR,
            max_air: MAX_AIR,
            eye_in_water: false,
            fully_frozen: false,
            frozen: 0.0,
            experience_progress: 0.0,
            experience_level: 0,
            effects: Vec::new(),
            selected: 0,
            slots: Default::default(),
            pop: [0; SLOTS],
            cooldowns: [Cooldown::default(); SLOTS],
            left_handed: false,
            attack_strength: 0.0,
            attack_delay: 0.0,
            target_alive: false,
            texts: HudTexts::default(),
        }
    }
}

impl HudRecord {
    /// `Player.getXpNeededForNextLevel`.
    pub fn experience_needed(&self) -> i32 {
        let level = self.experience_level;
        if level >= 30 {
            112 + (level - 30) * 9
        } else if level >= 15 {
            37 + (level - 15) * 5
        } else {
            7 + level * 2
        }
    }

    /// Takes in the player and the HUD's state as a tick left them. `target_alive` is
    /// the frame's: whether the crosshair is on something alive within the held item's reach.
    pub fn fill(
        &mut self,
        state: &HudState,
        player: &LocalPlayer,
        hardcore: bool,
        left_handed: bool,
        target_alive: bool,
    ) {
        use ruthenium_protocol::packets::play;
        let p = player;
        let mode = p.game_type();
        self.ticks = state.ticks;
        self.survival = mode == play::SURVIVAL || mode == play::ADVENTURE;
        self.spectator = mode == play::SPECTATOR;
        self.hardcore = hardcore;
        self.health = p.health;
        self.max_health = p.attributes.max_health.value() as f32;
        self.absorption = p.data.absorption;
        self.damage_cooldown = p.damage_cooldown;
        self.armor = p.attributes.armor.value().floor() as i32;
        self.food = p.food;
        self.saturation = p.saturation;
        self.air = p.data.air_supply;
        self.max_air = MAX_AIR;
        self.eye_in_water = p.is_eye_in_water();
        self.fully_frozen = p.data.ticks_frozen >= TICKS_TO_FREEZE;
        self.frozen = p.data.ticks_frozen.min(TICKS_TO_FREEZE) as f32 / TICKS_TO_FREEZE as f32;
        self.experience_progress = p.experience_progress;
        self.experience_level = p.experience_level;
        self.effects.clear();
        self.effects.extend(p.effects.iter().map(HudEffect::from));
        self.selected = p.inventory.selected;
        for (slot, held) in self.slots.iter_mut().enumerate() {
            let stack = if slot == OFF_HAND {
                p.inventory.get(crate::inventory::SLOT_OFFHAND)
            } else {
                p.inventory.get(slot)
            };
            // A stack is copied only when the slot holds another.
            if held != stack {
                *held = stack.clone();
            }
            self.cooldowns[slot] = if stack.is_empty() {
                Cooldown::default()
            } else {
                p.cooldowns.of(&stack.cooldown_group())
            };
        }
        self.pop = state.pop;
        self.left_handed = left_handed;
        self.attack_strength = p.attack_strength_scale(0.0);
        self.attack_delay = p.attack_strength_delay();
        self.target_alive = target_alive;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_need_more_experience_the_higher_they_are() {
        let needed = |level| {
            HudRecord {
                experience_level: level,
                ..HudRecord::default()
            }
            .experience_needed()
        };
        assert_eq!(
            [needed(0), needed(14), needed(15), needed(29)],
            [7, 35, 37, 107]
        );
        assert_eq!([needed(30), needed(31), needed(100)], [112, 121, 742]);
    }

    #[test]
    fn a_stack_that_came_or_grew_bulges_for_five_ticks() {
        let mut hud = HudState::default();
        let one = Stack {
            identity: 1,
            item: 5,
            count: 1,
            ..Stack::empty()
        };
        let two = Stack {
            identity: 2,
            count: 2,
            ..one.clone()
        };
        hud.slot_set(3, &Stack::empty(), &one);
        assert_eq!(hud.pop[3], 5);
        hud.slot_set(3, &two, &one);
        assert_eq!(hud.pop[3], 0, "a smaller stack is at rest");
        hud.slot_set(3, &one, &two);
        hud.slot_set(3, &two, &two);
        assert_eq!(hud.pop[3], 0, "and so is one of the same size");
        hud.slot_set(3, &one, &two);
        hud.slot_set(4, &one, &Stack::empty());
        assert_eq!((hud.pop[3], hud.pop[4]), (5, 0));
        hud.slot_replaced(3);
        assert_eq!(hud.pop[3], 0);
        // Slots beyond the hotbar and the off hand are not the HUD's.
        hud.slot_set(27, &Stack::empty(), &one);
        assert_eq!(hud.pop, [0; SLOTS]);
    }

    #[test]
    fn the_selected_items_name_shows_for_forty_ticks_after_it_changes() {
        use ruthenium_protocol::item::ComponentPatch;
        use std::sync::Arc;
        let mut hud = HudState::default();
        let mut p = LocalPlayer::new();
        let mut record = HudRecord::default();
        let mut shows = |hud: &HudState, p: &LocalPlayer| {
            record.fill(hud, p, false, false, false);
            (record.ticks, hud.highlight())
        };
        let stack = |identity, item, added: Vec<(i32, Component)>| Stack {
            identity,
            item,
            count: 1,
            patch: Arc::new(ComponentPatch {
                added,
                removed: Vec::new(),
                ..Default::default()
            }),
            ..Stack::empty()
        };
        // Nothing in the hand: nothing to name.
        hud.tick(&p);
        assert_eq!(shows(&hud, &p), (1, 0));
        // An item comes into the selected slot: its name shows, and counts down.
        p.inventory.set(0, stack(1, 5, Vec::new()));
        hud.tick(&p);
        assert_eq!(shows(&hud, &p), (2, 40));
        hud.tick(&p);
        hud.tick(&p);
        assert_eq!(shows(&hud, &p).1, 38);
        // The same item a little more worn is another stack of the same name.
        p.inventory.set(
            0,
            stack(2, 5, vec![(Kind::Damage.id() as i32, Component::Damage(3))]),
        );
        hud.tick(&p);
        assert_eq!(shows(&hud, &p).1, 37);
        // Given a name of its own, it is named again; so is another item.
        let named = Component::Other {
            kind: Kind::CustomName.id() as i32,
            data: vec![8, 0, 1, b'x'],
        };
        p.inventory
            .set(0, stack(3, 5, vec![(Kind::CustomName.id() as i32, named)]));
        hud.tick(&p);
        assert_eq!(shows(&hud, &p).1, 40);
        hud.tick(&p);
        p.inventory.set(0, stack(4, 6, Vec::new()));
        hud.tick(&p);
        assert_eq!(shows(&hud, &p).1, 40);
        // Another slot selected, with nothing in it.
        p.inventory.selected = 1;
        hud.tick(&p);
        assert_eq!(shows(&hud, &p).1, 0);
        // A stack that bulges comes to rest tick by tick.
        hud.slot_set(0, &Stack::empty(), &stack(5, 5, Vec::new()));
        hud.tick(&p);
        hud.tick(&p);
        record.fill(&hud, &p, false, false, false);
        assert_eq!(record.pop[0], 3);
        assert_eq!(record.selected, 1);
        assert_eq!(record.slots[0].item, 6);
    }

    #[test]
    fn a_cooldown_runs_out_with_the_tick() {
        let c = Cooldown {
            left: 5.0,
            of: 20.0,
        };
        assert_eq!(c.percent(0.0), 0.25);
        assert_eq!(c.percent(1.0), 0.2);
        assert_eq!(
            Cooldown {
                left: 30.0,
                of: 20.0
            }
            .percent(0.0),
            1.0
        );
        assert_eq!(
            Cooldown {
                left: 0.5,
                of: 20.0
            }
            .percent(1.0),
            0.0
        );
        assert_eq!(Cooldown::default().percent(0.5), 0.0);
    }
}
