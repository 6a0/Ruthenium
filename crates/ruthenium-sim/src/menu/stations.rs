//! The menus of the beacon, the brewing stand, the enchanting table, the grindstone and
//! the lectern as the client runs them (`BeaconMenu`, `BrewingStandMenu`,
//! `EnchantmentMenu`, `GrindstoneMenu`, `LecternMenu`): their slots in vanilla's
//! order, their rules, their data slots and their `quickMoveStack`.
//!
//! The client works out a grindstone's result itself, as vanilla's does
//! (`GrindstoneMenu.slotsChanged` runs on both sides: its `createResult` needs no
//! level), and clears an enchanting table's offers when its item can't be enchanted
//! (`EnchantmentMenu.slotsChanged`: the offers themselves only come from the server).

use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as K, Item};
use ruthenium_protocol::item::{Component, ComponentPatch, ItemStack};

use super::{Container, Inventory, Kind, Menu, Rule, Stack, Who, slot, standard_inventory};
use crate::item::Items;

/// `EnchantmentMenu`'s data slots: the three costs, the seed, the three enchantments
/// shown (`enchantClue`, -1 for none) and their levels (`levelClue`).
pub const ENCHANT_COSTS: usize = 0;
pub const ENCHANT_SEED: usize = 3;
pub const ENCHANT_CLUES: usize = 4;
pub const ENCHANT_LEVELS: usize = 7;

/// The items a menu builds stacks with (a grindstone's result), kept with the menu.
/// Menus compare equal whatever they hold.
#[derive(Clone, Default)]
pub struct MenuItems(pub Option<Arc<Items>>);

impl PartialEq for MenuItems {
    fn eq(&self, _: &MenuItems) -> bool {
        true
    }
}

impl std::fmt::Debug for MenuItems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "MenuItems(..)"
        } else {
            "MenuItems(-)"
        })
    }
}

/// The slots of a menu of this module (`addSlot` and `addStandardInventorySlots` in
/// each constructor), and how many of its own stacks and data slots it has.
pub(super) fn layout(kind: Kind, slots: &mut Vec<super::Slot>) -> (usize, usize) {
    let own = |slots: &mut Vec<super::Slot>, i: usize, x: i32, y: i32, rule: Rule| {
        slots.push(slot(Container::Own, i, x, y, rule));
    };
    match kind {
        Kind::Beacon => {
            own(slots, 0, 136, 110, Rule::BeaconPayment);
            standard_inventory(slots, 36, 137);
            (1, 3)
        }
        Kind::BrewingStand => {
            own(slots, 0, 56, 51, Rule::Potion);
            own(slots, 1, 79, 58, Rule::Potion);
            own(slots, 2, 102, 51, Rule::Potion);
            own(slots, 3, 79, 17, Rule::BrewingIngredient);
            own(slots, 4, 17, 17, Rule::BrewingFuel);
            standard_inventory(slots, 8, 84);
            (5, 4)
        }
        Kind::Enchantment => {
            own(slots, 0, 15, 47, Rule::EnchantItem);
            own(slots, 1, 35, 47, Rule::Lapis);
            standard_inventory(slots, 8, 84);
            (2, 10)
        }
        Kind::Grindstone => {
            own(slots, 0, 49, 19, Rule::GrindstoneInput);
            own(slots, 1, 49, 40, Rule::GrindstoneInput);
            // (A `ResultContainer`'s one stack, here the menu's third.)
            own(slots, 2, 129, 34, Rule::GrindstoneResult);
            standard_inventory(slots, 8, 84);
            (3, 0)
        }
        Kind::Lectern => {
            // The book, at the corner: the lectern's screen isn't a container screen.
            own(slots, 0, 0, 0, Rule::Plain);
            (1, 1)
        }
        Kind::Mount(m) => super::mount::layout(m, slots),
        _ => (0, 0),
    }
}

/// The data slots as a menu of `kind` starts them: an enchanting table shows no
/// enchantment (`enchantClue` and `levelClue` are -1).
pub(super) fn data(kind: Kind, n: usize) -> Vec<i32> {
    let mut d = vec![0; n];
    if kind == Kind::Enchantment {
        d[ENCHANT_CLUES..].fill(-1);
    }
    d
}

/// `Slot.mayPlace` of this module's slots.
pub(super) fn may_place(menu: &Menu, rule: Rule, stack: &Stack, who: &Who) -> bool {
    let tags = &who.items.tags;
    match rule {
        Rule::BeaconPayment => tags.items.is(stack, "minecraft:beacon_payment_items"),
        Rule::Potion => is_potion_input(stack, who),
        Rule::BrewingIngredient => who.recipes.contains("minecraft:brewing_reagent", stack),
        // `FuelSlot.mayPlaceItem`.
        Rule::BrewingFuel => stack.c.brewing_fuel,
        // A plain slot, of one.
        Rule::EnchantItem => true,
        Rule::Lapis => is_lapis(stack),
        Rule::GrindstoneInput => stack.c.damageable() || has_any_enchantments(stack),
        Rule::GrindstoneResult => false,
        Rule::MountEquipment(eq) => super::mount::may_place(menu, eq, stack, who),
        _ => true,
    }
}

/// `PotionIngredient.isPotionInput`.
fn is_potion_input(stack: &Stack, who: &Who) -> bool {
    who.recipes.contains("minecraft:brewing_input", stack)
        || who
            .items
            .tags
            .items
            .is(stack, "minecraft:brewing_potion_inputs")
}

fn is_lapis(stack: &Stack) -> bool {
    stack.item() == Item::LapisLazuli.id() as i32
}

/// `EnchantmentHelper.hasAnyEnchantments`.
pub fn has_any_enchantments(stack: &Stack) -> bool {
    !stack.c.enchantments.is_empty() || !stack.c.stored_enchantments.is_empty()
}

/// `ItemStack.isEnchantable`: it may be enchanted, and isn't yet.
pub fn is_enchantable(stack: &Stack) -> bool {
    stack.c.enchantable && !stack.c.enchantments_removed && stack.c.enchantments.is_empty()
}

impl Menu {
    /// `quickMoveStack`'s moves for this module's menus: where `stack` (slot `i`'s)
    /// goes. `false` ends it with nothing returned, as each menu's early
    /// `return ItemStack.EMPTY` does.
    pub(super) fn station_quick_move(
        &mut self,
        inv: &mut Inventory,
        stack: &mut Stack,
        i: usize,
        who: &Who,
    ) -> bool {
        match self.kind {
            Kind::Beacon => {
                if i == 0 {
                    self.move_item_stack_to(inv, stack, 1, 37, true, who)
                } else if !self.has_item(inv, 0)
                    && self.may_place(0, stack, who)
                    && stack.count == 1
                {
                    self.move_item_stack_to(inv, stack, 0, 1, false, who)
                } else if (1..28).contains(&i) {
                    self.move_item_stack_to(inv, stack, 28, 37, false, who)
                } else if (28..37).contains(&i) {
                    self.move_item_stack_to(inv, stack, 1, 28, false, who)
                } else {
                    self.move_item_stack_to(inv, stack, 1, 37, false, who)
                }
            }
            Kind::BrewingStand => {
                if i > 4 {
                    if stack.c.brewing_fuel {
                        // Into the fuel slot ends it, as does an ingredient that
                        // doesn't fit.
                        !(self.move_item_stack_to(inv, stack, 4, 5, false, who)
                            || self.may_place(3, stack, who)
                                && !self.move_item_stack_to(inv, stack, 3, 4, false, who))
                    } else if self.may_place(3, stack, who) {
                        self.move_item_stack_to(inv, stack, 3, 4, false, who)
                    } else if is_potion_input(stack, who) {
                        self.move_item_stack_to(inv, stack, 0, 3, false, who)
                    } else if (5..32).contains(&i) {
                        self.move_item_stack_to(inv, stack, 32, 41, false, who)
                    } else if (32..41).contains(&i) {
                        self.move_item_stack_to(inv, stack, 5, 32, false, who)
                    } else {
                        self.move_item_stack_to(inv, stack, 5, 41, false, who)
                    }
                } else {
                    self.move_item_stack_to(inv, stack, 5, 41, true, who)
                }
            }
            Kind::Enchantment => {
                if i == 0 || i == 1 {
                    self.move_item_stack_to(inv, stack, 2, 38, true, who)
                } else if is_lapis(stack) {
                    self.move_item_stack_to(inv, stack, 1, 2, true, who)
                } else if self.has_item(inv, 0) || !self.may_place(0, stack, who) {
                    false
                } else {
                    let one = stack.copy_with_count(1);
                    stack.shrink(1);
                    self.set(inv, 0, one);
                    true
                }
            }
            Kind::Grindstone => {
                let full = self.has_item(inv, 0) && self.has_item(inv, 1);
                if i == 2 {
                    self.move_item_stack_to(inv, stack, 3, 39, true, who)
                } else if i != 0 && i != 1 {
                    if !full {
                        self.move_item_stack_to(inv, stack, 0, 2, false, who)
                    } else if (3..30).contains(&i) {
                        self.move_item_stack_to(inv, stack, 30, 39, false, who)
                    } else {
                        // (From the hotbar; nothing else is left.)
                        !(30..39).contains(&i)
                            || self.move_item_stack_to(inv, stack, 3, 30, false, who)
                    }
                } else {
                    self.move_item_stack_to(inv, stack, 3, 39, false, who)
                }
            }
            // `LecternMenu.quickMoveStack` moves nothing.
            Kind::Lectern => false,
            Kind::Mount(_) => self.mount_quick_move(inv, stack, i, who),
            _ => false,
        }
    }

    /// `slotsChanged` of a menu's own container, after slot `i` was set: a
    /// grindstone's inputs make its result again; an enchanting table's item that can't
    /// be enchanted takes the offers away.
    pub(super) fn station_slot_changed(&mut self, i: usize) {
        match self.kind {
            Kind::Grindstone if i < 2 => {
                if let Some(items) = self.items.0.clone() {
                    self.own[2] = grindstone_result(&self.own[0], &self.own[1], &items);
                }
            }
            Kind::Enchantment if self.own[0].is_empty() || !is_enchantable(&self.own[0]) => {
                for k in 0..3 {
                    self.data[ENCHANT_COSTS + k] = 0;
                    self.data[ENCHANT_CLUES + k] = -1;
                    self.data[ENCHANT_LEVELS + k] = -1;
                }
            }
            _ => {}
        }
    }
}

/// For the live checks (`RUTHENIUM_MENU_LOG`): the slots a server's `set_content` or
/// `set_slot` would change, by the stacks' hashes as the server compares them
/// (`HashedStack.matches`), so a resend of what the client predicted reads as none.
pub fn log_resent(
    menu: &Menu,
    inv: &Inventory,
    from: usize,
    stacks: &[Stack],
    registries: &dyn ruthenium_protocol::hashed::SyncedRegistries,
) {
    let changed: Vec<String> = (stacks.iter().enumerate())
        .filter(|(k, s)| {
            let i = from + k;
            i < menu.slots.len()
                && super::hash(menu.item(inv, i), registries) != super::hash(s, registries)
        })
        .map(|(k, s)| format!("{}={}", from + k, super::describe(s)))
        .collect();
    if changed.is_empty() {
        eprintln!("[menu]   (no slot changes)");
    } else {
        eprintln!("[menu]   CHANGES slots [{}]", changed.join(" "));
    }
}

/// `EnchantmentHelper.getComponentType`: an enchanted book's are stored.
fn enchantment_kind(stack: &Stack) -> K {
    if stack.item() == Item::EnchantedBook.id() as i32 {
        K::StoredEnchantments
    } else {
        K::Enchantments
    }
}

/// A stack's enchantments of the kind it keeps them in (`getEnchantmentsForCrafting`),
/// or `None` when it has no such component (`get` is null).
fn enchantments_of(stack: &Stack) -> Option<Vec<(i32, i32)>> {
    match enchantment_kind(stack) {
        K::StoredEnchantments => (!stack
            .patch
            .removed
            .contains(&(K::StoredEnchantments.id() as i32)))
        .then(|| stack.c.stored_enchantments.clone()),
        _ => (!stack.c.enchantments_removed).then(|| stack.c.enchantments.clone()),
    }
}

/// A stack being made: its item, count and patch over the item's defaults.
struct Making<'a> {
    item: i32,
    count: i32,
    patch: ComponentPatch,
    items: &'a Items,
}

impl Making<'_> {
    fn of<'a>(s: &Stack, count: i32, items: &'a Items) -> Making<'a> {
        Making {
            item: s.item(),
            count,
            patch: (*s.patch).clone(),
            items,
        }
    }

    fn stack(&self) -> Stack {
        self.items.stack(&ItemStack {
            item: self.item,
            count: self.count,
            patch: self.patch.clone(),
        })
    }

    /// `PatchedDataComponentMap.set`: a value as the item's own is no change from it.
    fn set(&mut self, kind: K, value: Component, default: bool) {
        let id = kind.id() as i32;
        self.patch.removed.retain(|k| *k != id);
        match self.patch.added.iter().position(|(k, _)| *k == id) {
            Some(at) if default => {
                self.patch.added.remove(at);
            }
            Some(at) => self.patch.added[at].1 = value,
            None if default => {}
            None => self.patch.added.push((id, value)),
        }
    }

    fn defaults(&self) -> Arc<crate::item::Components> {
        self.items.defaults(self.item)
    }

    /// `EnchantmentHelper.updateEnchantments`: the enchantments as `f` leaves them, set
    /// on the stack, if it has the component at all.
    fn update_enchantments(&mut self, f: impl FnOnce(&mut Vec<(i32, i32)>)) -> Vec<(i32, i32)> {
        let now = self.stack();
        let Some(mut e) = enchantments_of(&now) else {
            return Vec::new();
        };
        f(&mut e);
        let kind = enchantment_kind(&now);
        let d = self.defaults();
        let default = match kind {
            K::StoredEnchantments => {
                self.item == Item::EnchantedBook.id() as i32 && same_set(&d.stored_enchantments, &e)
            }
            _ => !d.enchantments_removed && same_set(&d.enchantments, &e),
        };
        let value = match kind {
            K::StoredEnchantments => Component::StoredEnchantments(e.clone()),
            _ => Component::Enchantments(e.clone()),
        };
        self.set(kind, value, default);
        e
    }
}

/// Two enchantment maps equal as maps.
fn same_set(a: &[(i32, i32)], b: &[(i32, i32)]) -> bool {
    a.len() == b.len() && a.iter().all(|e| b.contains(e))
}

/// `GrindstoneMenu.computeResult`.
pub fn grindstone_result(input: &Stack, additional: &Stack, items: &Items) -> Stack {
    if input.is_empty() && additional.is_empty() {
        return Stack::empty();
    }
    if input.count > 1 || additional.count > 1 {
        return Stack::empty();
    }
    if input.is_empty() || additional.is_empty() {
        let item = if input.is_empty() { additional } else { input };
        if !has_any_enchantments(item) {
            return Stack::empty();
        }
        return remove_non_curses(Making::of(item, item.count, items));
    }
    merge_items(input, additional, items)
}

/// `GrindstoneMenu.mergeItems`.
fn merge_items(input: &Stack, additional: &Stack, items: &Items) -> Stack {
    if !input.same_item(additional) {
        return Stack::empty();
    }
    let max = |s: &Stack| s.c.max_damage.unwrap_or(0);
    let damage = |s: &Stack| s.c.damage.unwrap_or(0).clamp(0, max(s).max(0));
    let durability = max(input).max(max(additional));
    let remaining = (max(input) - damage(input))
        + (max(additional) - damage(additional))
        + durability * 5 / 100;
    let mut count = 1;
    if !input.c.damageable() {
        if input.max_stack_size() < 2 || !input.matches(additional) {
            return Stack::empty();
        }
        count = 2;
    }
    let mut m = Making::of(input, count, items);
    if input.c.damageable() {
        let d = m.defaults();
        m.set(
            K::MaxDamage,
            Component::MaxDamage(durability),
            d.max_damage == Some(durability),
        );
        let value = (durability - remaining).max(0).clamp(0, durability.max(0));
        m.set(K::Damage, Component::Damage(value), d.damage == Some(value));
    }
    // `mergeEnchantsFrom`: the other's enchantments, at the higher level, but a curse
    // already there stays as it is.
    let from = enchantments_of(additional).unwrap_or_default();
    let tags = &items.tags.enchantments;
    m.update_enchantments(|e| {
        for &(id, level) in &from {
            let curse = tags.contains("minecraft:curse", id);
            let here = e.iter().position(|(k, _)| *k == id);
            if (!curse || here.is_none()) && level > 0 {
                let level = level.min(255);
                match here {
                    Some(at) => e[at].1 = e[at].1.max(level),
                    None => e.push((id, level)),
                }
            }
        }
    });
    remove_non_curses(m)
}

/// `GrindstoneMenu.removeNonCursesFrom`: the curses alone stay; a book left with
/// nothing is a plain book again; the repair cost is one step per enchantment left.
fn remove_non_curses(mut m: Making<'_>) -> Stack {
    let items = m.items;
    let tags = &items.tags.enchantments;
    let left = m.update_enchantments(|e| e.retain(|(id, _)| tags.contains("minecraft:curse", *id)));
    if m.item == Item::EnchantedBook.id() as i32 && left.is_empty() {
        // `transmuteCopy`: the same patch on a book.
        m.item = Item::Book.id() as i32;
    }
    let mut cost: i32 = 0;
    for _ in &left {
        cost = (i64::from(cost) * 2 + 1).min(i64::from(i32::MAX)) as i32;
    }
    let mut bytes = Vec::new();
    ruthenium_protocol::wire::write_varint(&mut bytes, cost);
    m.set(
        K::RepairCost,
        Component::Other {
            kind: K::RepairCost.id() as i32,
            data: bytes,
        },
        cost == 0,
    );
    m.stack()
}

#[cfg(test)]
#[path = "stations_golden.rs"]
mod golden_m;

#[cfg(test)]
#[path = "stations_tests.rs"]
mod tests;
