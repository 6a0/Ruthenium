//! The player's inventory as the client holds it (`Inventory` and the slots of
//! `InventoryMenu`, container 0), the hands, and the item cooldowns.

use std::collections::HashMap;

use crate::item::{EquipmentSlot, Stack};

/// `InteractionHand`, by its network id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Main = 0,
    Off = 1,
}

impl Hand {
    pub const BOTH: [Hand; 2] = [Hand::Main, Hand::Off];

    pub fn id(self) -> i32 {
        self as i32
    }

    pub fn name(self) -> &'static str {
        match self {
            Hand::Main => "MAIN_HAND",
            Hand::Off => "OFF_HAND",
        }
    }
}

/// The 36 main slots (the hotbar is 0 to 8), then the equipment at 36 to 42
/// (`Inventory.EQUIPMENT_SLOT_MAPPING`: feet, legs, chest, head, off hand, body,
/// saddle). The menus over it are [`crate::menu`]'s.
#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    pub items: Vec<Stack>,
    pub equipment: [Stack; 7],
    /// The selected hotbar slot.
    pub selected: i32,
}

impl Default for Inventory {
    fn default() -> Inventory {
        Inventory {
            items: vec![Stack::empty(); 36],
            equipment: Default::default(),
            selected: 0,
        }
    }
}

/// `Inventory.SLOT_OFFHAND`.
pub const SLOT_OFFHAND: usize = 40;
/// `Inventory.getContainerSize`.
pub const SIZE: usize = 43;

impl Inventory {
    /// `getItem`.
    pub fn get(&self, slot: usize) -> &Stack {
        if slot < 36 {
            &self.items[slot]
        } else {
            self.equipment.get(slot - 36).unwrap_or(&self.equipment[0])
        }
    }

    pub fn get_mut(&mut self, slot: usize) -> Option<&mut Stack> {
        if slot < 36 {
            self.items.get_mut(slot)
        } else {
            self.equipment.get_mut(slot - 36)
        }
    }

    /// `setItem`.
    pub fn set(&mut self, slot: usize, stack: Stack) {
        if let Some(s) = self.get_mut(slot) {
            *s = stack;
        }
    }

    pub(crate) fn selected_index(&self) -> usize {
        usize::try_from(self.selected).unwrap_or(0).min(8)
    }

    /// The inventory slot of an equipment slot (the main hand is the selected slot).
    pub fn slot_of(&self, slot: EquipmentSlot) -> usize {
        match slot {
            EquipmentSlot::MainHand => self.selected_index(),
            EquipmentSlot::Feet => 36,
            EquipmentSlot::Legs => 37,
            EquipmentSlot::Chest => 38,
            EquipmentSlot::Head => 39,
            EquipmentSlot::OffHand => SLOT_OFFHAND,
            EquipmentSlot::Body => 41,
            EquipmentSlot::Saddle => 42,
        }
    }

    /// `getItemInHand`.
    pub fn hand(&self, hand: Hand) -> &Stack {
        match hand {
            Hand::Main => &self.items[self.selected_index()],
            Hand::Off => &self.equipment[SLOT_OFFHAND - 36],
        }
    }

    pub fn hand_mut(&mut self, hand: Hand) -> &mut Stack {
        match hand {
            Hand::Main => {
                let i = self.selected_index();
                &mut self.items[i]
            }
            Hand::Off => &mut self.equipment[SLOT_OFFHAND - 36],
        }
    }

    /// `setItemInHand`.
    pub fn set_hand(&mut self, hand: Hand, stack: Stack) {
        *self.hand_mut(hand) = stack;
    }

    /// `Inventory.add` for one stack that doesn't stack with others (the off-hand and
    /// armour swaps): the first free main slot, the selected one first. Whether it
    /// fitted.
    pub fn add(&mut self, stack: Stack) -> bool {
        if stack.is_empty() {
            return false;
        }
        let selected = self.selected_index();
        let free = if self.items[selected].is_empty() {
            Some(selected)
        } else {
            self.items.iter().position(Stack::is_empty)
        };
        match free {
            Some(i) => {
                self.items[i] = stack;
                true
            }
            None => false,
        }
    }
}

impl Inventory {
    /// `hasRemainingSpaceForItem`.
    fn has_room_for(&self, slot: usize, stack: &Stack) -> bool {
        let here = self.get(slot);
        !here.is_empty()
            && here.same_item_same_components(stack)
            && here.is_stackable()
            && here.count < here.max_stack_size().min(99)
    }

    /// `getSlotWithRemainingSpace`: the selected slot, the off hand, then the main
    /// slots in order.
    fn slot_with_room(&self, stack: &Stack) -> Option<usize> {
        let selected = self.selected_index();
        if self.has_room_for(selected, stack) {
            return Some(selected);
        }
        if self.has_room_for(SLOT_OFFHAND, stack) {
            return Some(SLOT_OFFHAND);
        }
        (0..36).find(|&i| self.has_room_for(i, stack))
    }

    /// `addResource(slot, stack)`: what of `stack` doesn't fit in `slot`.
    fn add_resource_at(&mut self, slot: usize, stack: &Stack) -> i32 {
        let mut count = stack.count;
        let mut here = self.get(slot).clone();
        if here.is_empty() {
            here = stack.copy_with_count(0);
        }
        let max_to_add = here.max_stack_size().min(99) - here.count;
        let to_add = count.min(max_to_add);
        if to_add == 0 {
            self.set(slot, here);
            return count;
        }
        count -= to_add;
        here.grow(to_add);
        self.set(slot, here);
        count
    }

    /// `Inventory.add(stack)` (any slot): a damaged tool goes whole into the first
    /// free slot; anything else joins like stacks and fills free slots. `stack` is
    /// left with what didn't fit (none with infinite materials). Whether any went in.
    pub fn add_stack(&mut self, stack: &mut Stack, infinite: bool) -> bool {
        if stack.is_empty() {
            return false;
        }
        if stack.is_damaged() {
            if let Some(free) = self.items.iter().position(Stack::is_empty) {
                // `copyAndClear`.
                let moved = stack.copy();
                stack.count = 0;
                self.items[free] = moved;
                return true;
            }
            if infinite {
                stack.count = 0;
                return true;
            }
            return false;
        }
        let mut last;
        loop {
            last = stack.count;
            let slot = self
                .slot_with_room(stack)
                .or_else(|| self.items.iter().position(Stack::is_empty));
            stack.count = match slot {
                Some(s) => self.add_resource_at(s, stack),
                None => stack.count,
            };
            if stack.is_empty() || stack.count >= last {
                break;
            }
        }
        if stack.count == last && infinite {
            stack.count = 0;
            return true;
        }
        stack.count < last
    }
}

/// `ItemCooldowns`: per cooldown group, the tick it started and the tick it ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cooldowns {
    cooldowns: HashMap<String, (i32, i32)>,
    tick_count: i32,
}

impl Cooldowns {
    /// `isOnCooldown`: `getCooldownPercent(item, 0) > 0`, in floats as vanilla.
    pub fn is_on_cooldown(&self, stack: &Stack) -> bool {
        self.percent(&stack.cooldown_group(), 0.0) > 0.0
    }

    /// `getCooldownPercent`.
    pub fn percent(&self, group: &str, a: f32) -> f32 {
        match self.cooldowns.get(group) {
            Some(&(start, end)) => {
                let duration = (end - start) as f32;
                let remaining = end as f32 - (self.tick_count as f32 + a);
                // Mth.clamp, with Java's Math.min: NaN (a cooldown of no length) stays
                // NaN, and NaN > 0 is false.
                let v = remaining / duration;
                if v < 0.0 {
                    0.0
                } else if v.is_nan() {
                    v
                } else {
                    v.min(1.0)
                }
            }
            None => 0.0,
        }
    }

    /// A group's cooldown as of this tick, for drawing between ticks.
    pub fn of(&self, group: &str) -> crate::hud::Cooldown {
        self.cooldowns
            .get(group)
            .map_or_else(Default::default, |&(start, end)| crate::hud::Cooldown {
                left: end as f32 - self.tick_count as f32,
                of: (end - start) as f32,
            })
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;
        let now = self.tick_count;
        self.cooldowns.retain(|_, &mut (_, end)| end > now);
    }

    /// `addCooldown`.
    pub fn add(&mut self, group: &str, ticks: i32) {
        self.cooldowns
            .insert(group.to_owned(), (self.tick_count, self.tick_count + ticks));
    }

    /// `removeCooldown`.
    pub fn remove(&mut self, group: &str) {
        self.cooldowns.remove(group);
    }

    pub fn groups(&self) -> impl Iterator<Item = &str> {
        self.cooldowns.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldowns_count_down() {
        let mut c = Cooldowns::default();
        let mut s = Stack::empty();
        s.item = 1;
        s.count = 1;
        c.add(&s.cooldown_group(), 2);
        assert!(c.is_on_cooldown(&s));
        c.tick();
        assert!(c.is_on_cooldown(&s));
        c.tick();
        assert!(!c.is_on_cooldown(&s));
        // A cooldown of no length is never on (0 / 0).
        c.add(&s.cooldown_group(), 0);
        assert!(!c.is_on_cooldown(&s));
    }
}
