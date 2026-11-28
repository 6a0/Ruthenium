//! `StonecutterMenu` and `StonecutterScreen` on the client: the recipes for the input
//! (the server's stonecutter list, `SelectableRecipe.SingleInputSet.selectByInput`), the
//! selected one (a data slot), choosing one (`clickMenuButton`, then
//! `container_button_click`), and the scrolling grid of buttons. The client has no
//! recipes to make the result with: choosing one clears it until the server sends it.

use ruthenium_protocol::recipe::{SlotDisplay, StonecutterEntry};

use super::{KindState, WorkData};
use crate::inventory::Inventory;
use crate::item::Stack;
use crate::menu::{Menu, Outcome, Who};

/// The stonecutter's state and its screen's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stonecutter {
    /// `recipesForInput`: indices into the session's list.
    pub recipes: Vec<usize>,
    /// `input`: the item last seen in the input (0: none).
    pub input_item: i32,
    pub scroll_offs: f32,
    pub scrolling: bool,
    pub start_index: i32,
    pub display_recipes: bool,
}

pub(super) fn cutter(m: &Menu) -> Option<&Stonecutter> {
    match &m.work.as_ref()?.kind {
        KindState::Stonecutter(s) => Some(s),
        _ => None,
    }
}

pub(super) fn cutter_mut(m: &mut Menu) -> Option<&mut Stonecutter> {
    match &mut m.work.as_mut()?.kind {
        KindState::Stonecutter(s) => Some(s),
        _ => None,
    }
}

/// `Ingredient.test`: the entry takes the stack's item.
fn takes(data: &WorkData, e: &StonecutterEntry, stack: &Stack) -> bool {
    !stack.is_empty() && data.item_in(&super::Holders::from_network(&e.input), stack)
}

/// `acceptsInput`.
pub fn accepts_input(data: &WorkData, stack: &Stack) -> bool {
    data.stonecutter.iter().any(|e| takes(data, e, stack))
}

/// The selected recipe (`selectedRecipeIndex`, data slot 0).
pub fn selected(m: &Menu) -> i32 {
    m.data.first().copied().unwrap_or(-1)
}

/// The input's `setChanged`: a new item (`slotsChanged`) lists its recipes, nothing
/// selected; then the screen hears of it (`containerChanged`).
pub(super) fn changed(m: &mut Menu, index: usize) {
    if index != 0 {
        return;
    }
    let input = m.own[0].clone();
    let data = m.work.as_ref().map(|w| w.data.clone()).unwrap_or_default();
    let item = if input.is_empty() { 0 } else { input.item };
    let Some(s) = cutter_mut(m) else {
        return;
    };
    if item != s.input_item {
        s.input_item = item;
        // `setupRecipeList`.
        s.recipes = if input.is_empty() {
            Vec::new()
        } else {
            (0..data.stonecutter.len())
                .filter(|&i| takes(&data, &data.stonecutter[i], &input))
                .collect()
        };
        m.data[0] = -1;
        m.own[1] = Stack::empty();
    }
    // `StonecutterScreen.containerChanged`.
    let has = has_input_item(m);
    let s = cutter_mut(m).expect("a stonecutter");
    s.display_recipes = has;
    s.scroll_offs = 0.0;
    s.start_index = 0;
}

/// `hasInputItem`.
pub fn has_input_item(m: &Menu) -> bool {
    !m.own[0].is_empty() && cutter(m).is_some_and(|s| !s.recipes.is_empty())
}

/// `clickMenuButton`: a recipe other than the selected one is chosen (and the result,
/// which the client can't make, clears). Whether the click counts (it is then sent).
pub fn click_menu_button(m: &mut Menu, id: i32) -> bool {
    if selected(m) == id {
        return false;
    }
    let count = cutter(m).map_or(0, |s| s.recipes.len()) as i32;
    if (0..count).contains(&id) {
        m.data[0] = id;
        // `setupResultSlot`: no recipe on the client.
        m.own[1] = Stack::empty();
    }
    true
}

/// The result's `onTake`: the stack taken is crafted (`onCraftedBy`: a map loses its
/// post-processing), one of the input goes, and the result (which the client can't make)
/// clears.
pub(super) fn on_take(m: &mut Menu, inv: &mut Inventory, _i: usize, taken: &mut Stack) {
    if let Some(w) = m.work.as_ref() {
        let items = w.data.items.clone();
        super::crafted_by(&items, taken);
    }
    // `inputSlot.remove(1)`: the container says it changed.
    let mut input = m.own[0].clone();
    if input.is_empty() {
        return;
    }
    let _ = input.split(1);
    m.own[0] = if input.count <= 0 {
        Stack::empty()
    } else {
        input
    };
    changed(m, 0);
    m.own[1] = Stack::empty();
    let _ = inv;
}

/// `StonecutterMenu.quickMoveStack`.
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
    let data = m.work.as_ref().map(|w| w.data.clone()).unwrap_or_default();
    let mut stack = m.get(inv, i);
    let clicked = stack.copy();
    let moved = if i == 1 {
        // `item.onCraftedBy` on the slot's own stack.
        super::crafted_by(&data.items, &mut stack);
        m.move_item_stack_to(inv, &mut stack, 2, 38, true, who)
    } else if i == 0 {
        m.move_item_stack_to(inv, &mut stack, 2, 38, false, who)
    } else if accepts_input(&data, &stack) {
        m.move_item_stack_to(inv, &mut stack, 0, 1, false, who)
    } else if (2..29).contains(&i) {
        m.move_item_stack_to(inv, &mut stack, 29, 38, false, who)
    } else if (29..38).contains(&i) {
        m.move_item_stack_to(inv, &mut stack, 2, 29, false, who)
    } else {
        true
    };
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    // An emptied slot is set empty; either way it is told it changed.
    if stack.is_empty() {
        m.set(inv, i, Stack::empty());
    }
    super::set_changed(m, inv, i);
    if stack.count == clicked.count {
        return Stack::empty();
    }
    let mut taken = stack.clone();
    m.on_take_stack(inv, i, &mut taken, who, out);
    if i == 1 && !taken.is_empty() {
        out.dropped.push(taken);
    }
    clicked
}

/// The stack a recipe's button shows (`resolveForFirstStack`).
pub fn display_stack(data: &WorkData, d: &SlotDisplay) -> Stack {
    let items = &data.items;
    match d {
        SlotDisplay::Item(id) => items.new_stack(*id, 1),
        SlotDisplay::ItemStack(s) => items.stack(s),
        SlotDisplay::Tag(set) => super::Holders::from_network(set)
            .entries(data.tags("minecraft:item"), &[])
            .first()
            .map_or_else(Stack::empty, |&id| items.new_stack(id, 1)),
        SlotDisplay::Composite(parts) => parts
            .iter()
            .map(|p| display_stack(data, p))
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        SlotDisplay::WithRemainder { input, .. } => display_stack(data, input),
        // (Not used by the stonecutter's recipes.)
        _ => Stack::empty(),
    }
}
