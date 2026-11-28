//! A mount's inventory as the client runs it (`HorseInventoryMenu`,
//! `NautilusInventoryMenu`, `AbstractMountInventoryMenu`): the saddle and body slots,
//! which are the mount's own equipment and take what it may wear, a chest of three rows
//! of so many columns as the server says, and the player's inventory.
//!
//! Which slots there are and what they take depend on the mount as the client knows
//! it (`canUseSlot`: alive, grown and tamed for some kinds; the entity type tags): the
//! session works that out as the screen opens ([`Mount::new`]).

use super::{Container, Inventory, Kind, Menu, Rule, Slot, Stack, Who, slot, standard_inventory};
use crate::item::{EquipmentSlot, MenuTags};

/// What a mount's inventory menu knows of the mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mount {
    /// Its `minecraft:entity_type` id.
    pub entity_type: i32,
    /// The chest's columns (`inventoryColumns`, from the server): none without one.
    pub columns: u8,
    /// An `AbstractNautilus` (its screen and slot icons), or a `Llama` (its armour is a
    /// carpet).
    pub nautilus: bool,
    pub llama: bool,
    /// `canUseSlot(SADDLE)`, `canUseSlot(BODY)`.
    pub saddle: bool,
    pub body: bool,
    /// The slots' `isActive` (and the screen's `shouldRenderSaddleSlot`,
    /// `shouldRenderArmorSlot`): hidden slots are neither drawn nor hovered.
    pub saddle_shown: bool,
    pub body_shown: bool,
}

/// What of a mount entity the menu reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MountState {
    pub alive: bool,
    pub baby: bool,
    /// `AbstractHorse.isTamed` (its flags' bit 2), or `TamableAnimal.isTame` (bit 4).
    pub tamed: bool,
}

impl Mount {
    /// The menu of the mount `name` (`minecraft:horse`...) of type `entity_type`,
    /// with `columns` of chest, if it is a mount with an inventory screen.
    pub fn new(
        name: &str,
        entity_type: i32,
        columns: i32,
        state: MountState,
        tags: &MenuTags,
    ) -> Option<Mount> {
        let name = name.trim_start_matches("minecraft:");
        let grown = state.alive && !state.baby && state.tamed;
        // `canUseSlot` of each kind: horses, the undead ones and llamas override it to
        // true; donkeys, mules and camels keep `AbstractHorse`'s for the saddle; a
        // nautilus needs both tamed.
        let (saddle, body) = match name {
            "horse" | "skeleton_horse" | "zombie_horse" | "llama" | "trader_llama" => (true, true),
            "donkey" | "mule" | "camel" | "camel_husk" => (grown, true),
            "nautilus" | "zombie_nautilus" => (grown, grown),
            _ => return None,
        };
        let nautilus = name.ends_with("nautilus");
        let llama = name.ends_with("llama");
        let is = |tag: &str| tags.entity_types.contains(tag, entity_type);
        let (saddle_shown, body_shown) = if nautilus {
            (saddle, body)
        } else {
            (
                saddle && is("minecraft:can_equip_saddle"),
                body && (is("minecraft:can_wear_horse_armor") || llama),
            )
        };
        Some(Mount {
            entity_type,
            columns: columns.clamp(0, 5) as u8,
            nautilus,
            llama,
            saddle,
            body,
            saddle_shown,
            body_shown,
        })
    }

    /// `getInventorySize`: the chest's slots.
    pub fn chest(&self) -> usize {
        usize::from(self.columns) * 3
    }
}

/// The menu's slots: saddle, body, the chest by rows, then the player's inventory.
pub(super) fn layout(m: Mount, slots: &mut Vec<Slot>) -> (usize, usize) {
    slots.push(slot(
        Container::Own,
        0,
        8,
        18,
        Rule::MountEquipment(EquipmentSlot::Saddle),
    ));
    slots.push(slot(
        Container::Own,
        1,
        8,
        36,
        Rule::MountEquipment(EquipmentSlot::Body),
    ));
    let columns = i32::from(m.columns);
    for y in 0..3 {
        for x in 0..columns {
            let i = 2 + (x + y * columns) as usize;
            slots.push(slot(
                Container::Own,
                i,
                80 + x * 18,
                18 + y * 18,
                Rule::Plain,
            ));
        }
    }
    standard_inventory(slots, 8, 84);
    (2 + m.chest(), 0)
}

/// `ArmorSlot.mayPlace` on the mount: `isEquippableInSlot`.
pub(super) fn may_place(menu: &Menu, eq: EquipmentSlot, stack: &Stack, who: &Who) -> bool {
    let Kind::Mount(m) = menu.kind else {
        return false;
    };
    let usable = match eq {
        EquipmentSlot::Saddle => m.saddle,
        EquipmentSlot::Body => m.body,
        _ => true,
    };
    stack.c.equippable.is_some_and(|e| e.slot == eq)
        && usable
        && who.items.can_be_equipped_by(stack, m.entity_type)
}

impl Menu {
    /// Whether slot `i` is there to see and point at (`Slot.isActive`).
    pub fn slot_active(&self, i: usize) -> bool {
        match (self.kind, i) {
            (Kind::Mount(m), 0) => m.saddle_shown,
            (Kind::Mount(m), 1) => m.body_shown,
            _ => true,
        }
    }

    /// `AbstractMountInventoryMenu.quickMoveStack`'s moves.
    pub(super) fn mount_quick_move(
        &mut self,
        inv: &mut Inventory,
        stack: &mut Stack,
        i: usize,
        who: &Who,
    ) -> bool {
        let Kind::Mount(m) = self.kind else {
            return false;
        };
        let start = 2 + m.chest();
        let n = self.slots.len();
        if i < start {
            return self.move_item_stack_to(inv, stack, start, n, true, who);
        }
        if self.may_place(1, stack, who) && !self.has_item(inv, 1) {
            return self.move_item_stack_to(inv, stack, 1, 2, false, who);
        }
        if self.may_place(0, stack, who) && !self.has_item(inv, 0) {
            return self.move_item_stack_to(inv, stack, 0, 1, false, who);
        }
        if m.chest() != 0 && self.move_item_stack_to(inv, stack, 2, start, false, who) {
            return true;
        }
        // Between the inventory and the hotbar, and then nothing is returned either way.
        let end = start + 27;
        if (end..end + 9).contains(&i) {
            self.move_item_stack_to(inv, stack, start, end, false, who);
        } else if (start..end).contains(&i) {
            self.move_item_stack_to(inv, stack, end, end + 9, false, who);
        }
        false
    }
}
