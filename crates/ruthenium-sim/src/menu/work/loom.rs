//! `LoomMenu` and `LoomScreen` on the client: the banner, dye and pattern slots, the
//! patterns a pattern item (or none) offers (`getSelectablePatterns`: the
//! `banner_pattern` registry's tags), the selected one (a data slot), choosing one
//! (`clickMenuButton`, then `container_button_click`), the banner it makes (worked out
//! by the client as vanilla's does), and the screen's scrolling grid.

use ruthenium_data::v26_3::registries::DataComponentType as ComponentKind;
use ruthenium_protocol::item::{Component, TrimPart};

use super::{KindState, WorkData};
use crate::inventory::Inventory;
use crate::item::Stack;
use crate::menu::{Menu, Outcome, Who};

/// `BannerPatternLayers`' most layers a loom adds to.
pub const MAX_LAYERS: usize = 6;

/// The loom's state and its screen's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loom {
    /// `selectablePatterns`: `banner_pattern` registry ids.
    pub patterns: Vec<i32>,
    // `LoomScreen`'s fields.
    pub display_patterns: bool,
    pub has_max_patterns: bool,
    pub scroll_offs: f32,
    pub scrolling: bool,
    pub start_row: i32,
    pub banner: Stack,
    pub dye: Stack,
    pub pattern: Stack,
}

pub(super) fn loom(m: &Menu) -> Option<&Loom> {
    match &m.work.as_ref()?.kind {
        KindState::Loom(l) => Some(l),
        _ => None,
    }
}

pub(super) fn loom_mut(m: &mut Menu) -> Option<&mut Loom> {
    match &mut m.work.as_mut()?.kind {
        KindState::Loom(l) => Some(l),
        _ => None,
    }
}

/// `instanceof BannerItem`.
pub fn is_banner(s: &Stack) -> bool {
    !s.is_empty() && s.c.is("BannerItem")
}

/// `isDyeItem`.
pub fn is_dye(data: &WorkData, s: &Stack) -> bool {
    data.item_is(s, "minecraft:loom_dyes") && s.c.work.dye.is_some()
}

/// `isPatternItem`.
pub fn is_pattern(data: &WorkData, s: &Stack) -> bool {
    data.item_is(s, "minecraft:loom_patterns") && s.c.work.provides_banner_patterns.is_some()
}

/// The selected pattern (`selectedBannerPatternIndex`, data slot 0).
pub fn selected(m: &Menu) -> i32 {
    m.data.first().copied().unwrap_or(-1)
}

/// A banner's layers as its patch gives them (none by default).
fn layers(s: &Stack) -> Vec<(TrimPart, i32)> {
    match s.patch.get(ComponentKind::BannerPatterns) {
        Some(Component::BannerPatterns(l)) => l.clone(),
        _ => Vec::new(),
    }
}

/// `getSelectablePatterns`: without a pattern item, those that need none.
fn selectable(data: &WorkData, pattern: &Stack) -> Vec<i32> {
    let tags = data.tags("minecraft:banner_pattern");
    if pattern.is_empty() {
        return tags
            .and_then(|t| t.get("minecraft:no_item_required"))
            .cloned()
            .unwrap_or_default();
    }
    pattern
        .c
        .work
        .provides_banner_patterns
        .as_ref()
        .map(|h| h.entries(tags, &data.banner_pattern_names()))
        .unwrap_or_default()
}

/// The inputs' `setChanged`: `slotsChanged`, then the screen hears of it. The result's:
/// only the screen.
pub(super) fn changed(m: &mut Menu, index: usize) {
    if index < 3 {
        slots_changed(m);
    }
    container_changed(m);
}

/// `LoomMenu.slotsChanged`.
fn slots_changed(m: &mut Menu) {
    let data = m.work.as_ref().map(|w| w.data.clone()).unwrap_or_default();
    let (banner, dye, pattern) = (m.own[0].clone(), m.own[1].clone(), m.own[2].clone());
    if banner.is_empty() || dye.is_empty() {
        result_set(m, Stack::empty());
        if let Some(l) = loom_mut(m) {
            l.patterns.clear();
        }
        m.data[0] = -1;
        return;
    }
    let sel = selected(m);
    let previous = loom(m).map(|l| l.patterns.clone()).unwrap_or_default();
    let valid = sel >= 0 && (sel as usize) < previous.len();
    let patterns = selectable(&data, &pattern);
    if let Some(l) = loom_mut(m) {
        l.patterns = patterns.clone();
    }
    let shown = if patterns.len() == 1 {
        m.data[0] = 0;
        Some(patterns[0])
    } else if !valid {
        m.data[0] = -1;
        None
    } else {
        let value = previous[sel as usize];
        match patterns.iter().position(|&p| p == value) {
            Some(i) => {
                m.data[0] = i as i32;
                Some(value)
            }
            None => {
                m.data[0] = -1;
                None
            }
        }
    };
    match shown {
        Some(p) if layers(&banner).len() >= MAX_LAYERS => {
            m.data[0] = -1;
            result_set(m, Stack::empty());
            let _ = p;
        }
        Some(p) => setup_result(m, p),
        None => result_set(m, Stack::empty()),
    }
}

/// `resultSlot.set`: the output container says it changed (the screen hears it).
fn result_set(m: &mut Menu, s: Stack) {
    m.own[3] = s;
    container_changed(m);
}

/// `setupResultSlot`: one banner with the pattern added in the dye's colour.
fn setup_result(m: &mut Menu, pattern: i32) {
    let data = m.work.as_ref().map(|w| w.data.clone()).unwrap_or_default();
    let (banner, dye) = (m.own[0].clone(), m.own[1].clone());
    let mut result = Stack::empty();
    if !banner.is_empty()
        && !dye.is_empty()
        && let Some(colour) = dye.c.work.dye
    {
        let mut l = layers(&banner);
        l.push((TrimPart::Id(pattern), i32::from(colour)));
        result = super::set_component(
            &data.items,
            &banner.copy_with_count(1),
            ComponentKind::BannerPatterns,
            Component::BannerPatterns(l),
            false,
        );
    }
    if !result.matches(&m.own[3]) {
        result_set(m, result);
    }
}

/// `clickMenuButton`: a pattern of the list is chosen and the banner made.
pub fn click_menu_button(m: &mut Menu, id: i32) -> bool {
    let patterns = loom(m).map(|l| l.patterns.clone()).unwrap_or_default();
    if id >= 0 && (id as usize) < patterns.len() {
        m.data[0] = id;
        setup_result(m, patterns[id as usize]);
        true
    } else {
        false
    }
}

/// The result's `onTake`: a banner and a dye go; without one of them nothing is selected.
pub(super) fn on_take(m: &mut Menu, inv: &mut Inventory, i: usize) {
    for slot in [0, 1] {
        let mut s = m.own[slot].clone();
        if !s.is_empty() {
            let _ = s.split(1);
            m.own[slot] = if s.count <= 0 { Stack::empty() } else { s };
            changed(m, slot);
        }
    }
    if m.own[0].is_empty() || m.own[1].is_empty() {
        m.data[0] = -1;
    }
    // (`Slot.onTake`: the output's `setChanged`.)
    super::set_changed(m, inv, i);
}

/// `LoomScreen.containerChanged`.
fn container_changed(m: &mut Menu) {
    let (banner, dye, pattern) = (m.own[0].clone(), m.own[1].clone(), m.own[2].clone());
    let Some(l) = loom_mut(m) else {
        return;
    };
    l.has_max_patterns = layers(&banner).len() >= MAX_LAYERS;
    if !banner.matches(&l.banner) || !dye.matches(&l.dye) || !pattern.matches(&l.pattern) {
        l.display_patterns =
            !banner.is_empty() && !dye.is_empty() && !l.has_max_patterns && !l.patterns.is_empty();
    }
    if l.start_row >= total_rows(l) {
        l.start_row = 0;
        l.scroll_offs = 0.0;
    }
    l.banner = banner.copy();
    l.dye = dye.copy();
    l.pattern = pattern.copy();
}

/// `totalRowCount`.
pub fn total_rows(l: &Loom) -> i32 {
    (l.patterns.len() as i32 + 3) / 4
}

/// `LoomMenu.quickMoveStack`.
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
    let moved = if i == 3 {
        m.move_item_stack_to(inv, &mut stack, 4, 40, true, who)
    } else if i > 3 {
        if is_banner(&stack) {
            m.move_item_stack_to(inv, &mut stack, 0, 1, false, who)
        } else if is_dye(&data, &stack) {
            m.move_item_stack_to(inv, &mut stack, 1, 2, false, who)
        } else if is_pattern(&data, &stack) {
            m.move_item_stack_to(inv, &mut stack, 2, 3, false, who)
        } else if (4..31).contains(&i) {
            m.move_item_stack_to(inv, &mut stack, 31, 40, false, who)
        } else if (31..40).contains(&i) {
            m.move_item_stack_to(inv, &mut stack, 4, 31, false, who)
        } else {
            true
        }
    } else {
        m.move_item_stack_to(inv, &mut stack, 4, 40, false, who)
    };
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    super::quick_move_tail(m, inv, i, stack, &clicked, who, out)
}
