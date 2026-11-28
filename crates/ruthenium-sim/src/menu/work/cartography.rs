//! `CartographyTableMenu` on the client: the map and paper/map/glass pane slots, and a
//! result the client never makes (its level access is empty) but takes away when an
//! input goes; taking it uses one of each.

use crate::inventory::Inventory;
use crate::item::Stack;
use crate::menu::{Menu, Outcome, Who};

/// The additional slot's items: paper, an empty map, a glass pane.
pub fn is_addition(s: &Stack) -> bool {
    matches!(
        s.c.name,
        "minecraft:paper" | "minecraft:map" | "minecraft:glass_pane"
    ) && !s.is_empty()
}

/// Either container's `setChanged` (`slotsChanged`): a result without both inputs goes
/// (`removeItemNoUpdate`).
pub(super) fn changed(m: &mut Menu) {
    let (map, addition, result) = (&m.own[0], &m.own[1], &m.own[2]);
    if !result.is_empty() && (map.is_empty() || addition.is_empty()) {
        m.own[2] = Stack::empty();
    }
}

/// The result's `onTake`: one of each input goes; a map taken loses its post-processing
/// (`onCraftedBy`).
pub(super) fn on_take(m: &mut Menu, inv: &mut Inventory, i: usize, taken: &mut Stack) {
    for slot in [0, 1] {
        let mut s = m.own[slot].clone();
        if !s.is_empty() {
            let _ = s.split(1);
            m.own[slot] = if s.count <= 0 { Stack::empty() } else { s };
            changed(m);
        }
    }
    if let Some(w) = m.work.as_ref() {
        let items = w.data.items.clone();
        super::crafted_by(&items, taken);
    }
    super::set_changed(m, inv, i);
}

/// `CartographyTableMenu.quickMoveStack`.
pub(super) fn quick_move(
    m: &mut Menu,
    inv: &mut Inventory,
    i: usize,
    who: &Who,
    out: &mut Outcome,
) -> Stack {
    if !m.has_item(inv, i) {
        return Stack::empty();
    }
    let mut stack = m.get(inv, i);
    let clicked = stack.copy();
    let moved = if i == 2 {
        if let Some(w) = m.work.as_ref() {
            let items = w.data.items.clone();
            super::crafted_by(&items, &mut stack);
        }
        m.move_item_stack_to(inv, &mut stack, 3, 39, true, who)
    } else if i != 1 && i != 0 {
        if stack.c.work.map_id {
            m.move_item_stack_to(inv, &mut stack, 0, 1, false, who)
        } else if !is_addition(&stack) {
            if (3..30).contains(&i) {
                m.move_item_stack_to(inv, &mut stack, 30, 39, false, who)
            } else if (30..39).contains(&i) {
                m.move_item_stack_to(inv, &mut stack, 3, 30, false, who)
            } else {
                true
            }
        } else {
            m.move_item_stack_to(inv, &mut stack, 1, 2, false, who)
        }
    } else {
        m.move_item_stack_to(inv, &mut stack, 3, 39, false, who)
    };
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    if stack.is_empty() {
        m.set(inv, i, Stack::empty());
    }
    super::set_changed(m, inv, i);
    if stack.count == clicked.count {
        return Stack::empty();
    }
    let mut taken = stack;
    m.on_take_stack(inv, i, &mut taken, who, out);
    clicked
}
