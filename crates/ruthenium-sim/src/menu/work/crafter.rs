//! `CrafterMenu` and `CrafterScreen` on the client: the 3x3 grid, whose slots a click on
//! an empty one switches off or on (`container_slot_state_changed`, its data slots 0 to
//! 8), the powered state (data slot 9), and the result the server shows, which nothing
//! can take.

use crate::inventory::Inventory;
use crate::item::Stack;
use crate::menu::{Input, Menu, Outcome, Who};

/// `isSlotDisabled`.
pub fn is_disabled(m: &Menu, slot: i32) -> bool {
    (0..9).contains(&slot) && m.data.get(slot as usize) == Some(&1)
}

/// `isPowered`.
pub fn is_powered(m: &Menu) -> bool {
    m.data.get(9) == Some(&1)
}

/// `setSlotState`.
pub fn set_slot_state(m: &mut Menu, slot: usize, enabled: bool) {
    if let Some(d) = m.data.get_mut(m.slots[slot].index) {
        *d = if enabled { 0 } else { 1 };
    }
}

/// `CrafterScreen.slotClicked`: a click on an empty grid slot switches it (a press off
/// with nothing carried, a press or a hotbar key with something on it); the screen then
/// clicks as usual. The slot switched and its new state, if any.
pub fn toggle(
    m: &Menu,
    inv: &Inventory,
    slot: i32,
    button: i32,
    input: Input,
) -> Option<(usize, bool)> {
    let i = usize::try_from(slot).ok().filter(|&i| i < 9)?;
    if !m.item(inv, i).is_empty() {
        return None;
    }
    match input {
        Input::Pickup if is_disabled(m, slot) => Some((i, true)),
        Input::Pickup if m.carried.is_empty() => Some((i, false)),
        Input::Swap if is_disabled(m, slot) && !inv.get(button as usize).is_empty() => {
            Some((i, true))
        }
        _ => None,
    }
}

/// `CrafterMenu.quickMoveStack`.
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
    let moved = if i < 9 {
        m.move_item_stack_to(inv, &mut stack, 9, 45, true, who)
    } else {
        m.move_item_stack_to(inv, &mut stack, 0, 9, false, who)
    };
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    super::quick_move_tail(m, inv, i, stack, &clicked, who, out)
}
