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
/// saddle); the crafting grid and result of the inventory menu; the stack the cursor
/// carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    pub items: Vec<Stack>,
    pub equipment: [Stack; 7],
    /// The selected hotbar slot.
    pub selected: i32,
    /// Menu slots 0 (result) to 4 (the 2x2 grid).
    pub crafting: [Stack; 5],
    pub carried: Stack,
    pub state_id: i32,
}

impl Default for Inventory {
    fn default() -> Inventory {
        Inventory {
            items: vec![Stack::empty(); 36],
            equipment: Default::default(),
            selected: 0,
            crafting: Default::default(),
            carried: Stack::empty(),
            state_id: 0,
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

    fn selected_index(&self) -> usize {
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

    /// `InventoryMenu`'s slot `slot` (container 0): where it keeps its stack.
    fn menu_slot(&mut self, slot: i32) -> Option<&mut Stack> {
        let i = usize::try_from(slot).ok()?;
        match i {
            0..=4 => self.crafting.get_mut(i),
            // Head first: the armor slots go 39 down to 36.
            5..=8 => self.get_mut(39 - (i - 5)),
            9..=35 => self.get_mut(i),
            36..=44 => self.get_mut(i - 36),
            45 => self.get_mut(SLOT_OFFHAND),
            _ => None,
        }
    }

    /// `InventoryMenu.setItem` (`ClientboundContainerSetSlotPacket` for container 0).
    pub fn set_menu_slot(&mut self, slot: i32, state_id: i32, stack: Stack) {
        if let Some(s) = self.menu_slot(slot) {
            *s = stack;
        }
        self.state_id = state_id;
    }

    /// `initializeContents` (`ClientboundContainerSetContentPacket` for container 0).
    pub fn initialize(&mut self, state_id: i32, items: Vec<Stack>, carried: Stack) {
        for (i, s) in items.into_iter().enumerate() {
            if let Some(slot) = self.menu_slot(i as i32) {
                *slot = s;
            }
        }
        self.carried = carried;
        self.state_id = state_id;
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
    fn menu_slots_map_to_the_inventory() {
        let mut inv = Inventory::default();
        let mut a = Stack::empty();
        a.item = 5;
        a.count = 1;
        inv.set_menu_slot(36, 3, a.clone());
        assert_eq!(inv.items[0].item, 5);
        inv.set_menu_slot(45, 4, a.clone());
        assert_eq!(inv.hand(Hand::Off).item, 5);
        inv.set_menu_slot(5, 5, a.clone());
        assert_eq!(inv.get(39).item, 5);
        assert_eq!(inv.state_id, 5);
        inv.selected = 0;
        assert_eq!(inv.hand(Hand::Main).item, 5);
    }

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
