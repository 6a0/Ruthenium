//! `AnvilMenu` and `AnvilScreen` on the client: the result and its cost worked out as
//! vanilla's client does (`createResult`: repairs with material or a second item,
//! enchantments joined, renaming), the cost's data slot, the result's rule (enough
//! levels, a cost above 0), taking it, and the name box (`slotChanged` fills it with the
//! input's name; each change it makes sends `rename_item` as `onNameChanged` does).

use ruthenium_data::v26_3::registries::DataComponentType as ComponentKind;
use ruthenium_protocol::item::Component;

use super::edit_box::EditBox;
use super::{KindState, State, WorkData};
use crate::item::Stack;
use crate::menu::{Menu, Who};

/// `AnvilMenu.MAX_NAME_LENGTH`.
pub const MAX_NAME_LENGTH: usize = 50;

/// The anvil's own state and its screen's.
#[derive(Debug, Clone, PartialEq)]
pub struct Anvil {
    /// `itemName` (`None` until the box first says something).
    pub item_name: Option<String>,
    pub repair_item_count_cost: i32,
    pub only_renaming: bool,
    /// `lastSlots[0]`: what the screen last heard slot 0 hold.
    pub last_input: Stack,
    /// The name box (`setCanLoseFocus(false)`: always focused).
    pub name: EditBox,
    /// When the box took the focus (its caret blinks from then).
    pub focused: Option<std::time::Instant>,
}

impl Default for Anvil {
    fn default() -> Anvil {
        let mut name = EditBox::new(MAX_NAME_LENGTH);
        // `subInit`: editable only with an input, which a new menu hasn't.
        name.editable = false;
        Anvil {
            item_name: None,
            repair_item_count_cost: 0,
            only_renaming: false,
            last_input: Stack::empty(),
            name,
            focused: Some(std::time::Instant::now()),
        }
    }
}

fn anvil(m: &Menu) -> Option<&Anvil> {
    match &m.work.as_ref()?.kind {
        KindState::Anvil(a) => Some(a),
        _ => None,
    }
}

fn anvil_mut(m: &mut Menu) -> Option<&mut Anvil> {
    match &mut m.work.as_mut()?.kind {
        KindState::Anvil(a) => Some(a),
        _ => None,
    }
}

fn state(m: &Menu) -> &State {
    m.work.as_ref().expect("a workstation's menu has its state")
}

/// The input container's `setChanged` (`ItemCombinerMenu.slotsChanged`): the screen
/// hears of it (`broadcastChanges`), then an input's change makes the result again. The
/// result's container only does the former.
pub(super) fn changed(m: &mut Menu, index: usize) {
    broadcast(m);
    if index < 2 {
        create_result(m);
    }
}

/// `broadcastChanges` as the anvil's screen hears it: slot 0 changed since it last
/// heard (`AnvilScreen.slotChanged`) puts the input's name in the box.
fn broadcast(m: &mut Menu) {
    let input = m.own[0].clone();
    let Some(a) = anvil_mut(m) else {
        return;
    };
    if a.last_input.matches(&input) {
        return;
    }
    a.last_input = input.copy();
    let data = state(m).data.clone();
    let text = if input.is_empty() {
        String::new()
    } else {
        data.hover_name(&input)
    };
    let a = anvil_mut(m).expect("an anvil");
    let said = a.name.set_value(&text);
    a.name.editable = !input.is_empty();
    on_name_changed(m, &said);
}

/// `AnvilScreen.onNameChanged`: the input's own name means no name; a name the menu
/// takes is sent to the server.
pub fn on_name_changed(m: &mut Menu, name: &str) {
    let input = m.own[0].clone();
    if input.is_empty() {
        return;
    }
    let data = state(m).data.clone();
    let mut name = name.to_owned();
    if input.c.custom_name.is_none() && name == data.hover_name(&input) {
        name.clear();
    }
    if set_item_name(m, &name) {
        let packet = ruthenium_protocol::packets::play::rename_item(&name);
        if let Some(w) = m.work.as_mut() {
            w.outbox.push(packet);
        }
    }
}

/// `AnvilMenu.setItemName`: a new name (filtered, at most 50 long) makes the result
/// again. Whether it was taken.
pub fn set_item_name(m: &mut Menu, name: &str) -> bool {
    let validated = super::filter_text(name);
    if validated.encode_utf16().count() > MAX_NAME_LENGTH {
        return false;
    }
    let Some(a) = anvil_mut(m) else {
        return false;
    };
    if a.item_name.as_deref() == Some(validated.as_str()) {
        return false;
    }
    a.item_name = Some(validated);
    create_result(m);
    true
}

/// `AnvilMenu.mayPickup`: enough levels (or infinite materials) and a cost above 0.
pub(super) fn may_pickup(m: &Menu, who: &Who) -> bool {
    let cost = m.data.first().copied().unwrap_or(0);
    let p = state(m).player;
    (who.creative || p.xp_level >= cost) && cost > 0
}

/// `AnvilMenu.onTake`: the levels go, the material or the second item is used up, and
/// the input goes.
pub(super) fn on_take(m: &mut Menu, who: &Who) {
    let cost = m.data.first().copied().unwrap_or(0);
    if !who.creative
        && let Some(w) = m.work.as_mut()
    {
        // (`giveExperienceLevels`: the client's own count drops at once, to no less
        // than 0.)
        w.player.xp_level = (w.player.xp_level - cost).max(0);
    }
    let (repair, renaming) =
        anvil(m).map_or((0, false), |a| (a.repair_item_count_cost, a.only_renaming));
    if repair > 0 {
        let mut addition = m.own[1].clone();
        if !addition.is_empty() && addition.count > repair {
            addition.shrink(repair);
            input_set(m, 1, addition);
        } else {
            input_set(m, 1, Stack::empty());
        }
    } else if !renaming {
        input_set(m, 1, Stack::empty());
    }
    if let Some(d) = m.data.first_mut() {
        *d = 0;
    }
    input_set(m, 0, Stack::empty());
}

/// `inputSlots.setItem`: the stack goes in and the container says it changed.
fn input_set(m: &mut Menu, index: usize, stack: Stack) {
    m.own[index] = stack;
    changed(m, index);
}

fn set_cost(m: &mut Menu, cost: i32) {
    if let Some(d) = m.data.first_mut() {
        *d = cost;
    }
}

/// `AnvilMenu.calculateIncreasedRepairCost`.
pub fn increased_repair_cost(base: i32) -> i32 {
    (i64::from(base) * 2 + 1).min(i64::from(i32::MAX)) as i32
}

/// `getEnchantmentsForCrafting`: a book's stored enchantments, else its own.
fn crafting_enchantments(s: &Stack) -> Vec<(i32, i32)> {
    if is_book(s) {
        s.c.work.stored_enchantments.clone().unwrap_or_default()
    } else {
        s.c.enchantments.clone()
    }
}

fn is_book(s: &Stack) -> bool {
    s.c.name == "minecraft:enchanted_book"
}

/// `EnchantmentHelper.canStoreEnchantments`.
fn can_store_enchantments(s: &Stack) -> bool {
    if is_book(s) {
        s.c.work.stored_enchantments.is_some()
    } else {
        s.c.work.has_enchantments
    }
}

fn damage(s: &Stack) -> i32 {
    s.c.damage.unwrap_or(0).clamp(0, max_damage(s))
}

fn max_damage(s: &Stack) -> i32 {
    s.c.max_damage.unwrap_or(0)
}

/// `setDamageValue`: clamped; the item's own default leaves the patch without it.
fn with_damage(data: &WorkData, s: &Stack, value: i32) -> Stack {
    let value = value.clamp(0, max_damage(s));
    let default = data.items.defaults(s.item).damage == Some(value);
    super::set_component(
        &data.items,
        s,
        ComponentKind::Damage,
        Component::Damage(value),
        default,
    )
}

/// `Enchantment.areCompatible`.
fn compatible(data: &WorkData, a: i32, b: i32) -> bool {
    let e = &data.items.enchantments;
    let tags = data.tags("minecraft:enchantment");
    let excludes = |x: i32, y: i32| {
        e.get(x)
            .and_then(|d| d.exclusive_set.as_ref())
            .is_some_and(|set| set.contains(y, e.name(y).unwrap_or(""), tags))
    };
    a != b && !excludes(a, b) && !excludes(b, a)
}

/// `AnvilMenu.createResult`.
pub fn create_result(m: &mut Menu) {
    let w = state(m);
    let data = w.data.clone();
    let creative = w.player.creative;
    let item_name = anvil(m).and_then(|a| a.item_name.clone());
    let input = m.own[0].clone();
    if let Some(a) = anvil_mut(m) {
        a.only_renaming = false;
    }
    set_cost(m, 1);
    let mut price: i64 = 0;
    let mut tax: i64 = 0;
    let mut naming_cost: i64 = 0;
    if input.is_empty() || !can_store_enchantments(&input) {
        m.own[2] = Stack::empty();
        set_cost(m, 0);
        return;
    }
    let mut result = input.copy();
    let addition = m.own[1].clone();
    let mut enchantments = crafting_enchantments(&result);
    tax += i64::from(input.c.work.repair_cost) + i64::from(addition.c.work.repair_cost);
    let mut repair_count = 0;
    let fail = |m: &mut Menu| {
        m.own[2] = Stack::empty();
        set_cost(m, 0);
    };
    if !addition.is_empty() {
        let using_book = addition.c.work.stored_enchantments.is_some();
        let repairs = input
            .c
            .work
            .repairable
            .as_ref()
            .is_some_and(|r| data.item_in(r, &addition));
        if result.c.damageable() && repairs {
            let mut amount = damage(&result).min(max_damage(&result) / 4);
            if amount <= 0 {
                if let Some(a) = anvil_mut(m) {
                    a.repair_item_count_cost = 0;
                }
                fail(m);
                return;
            }
            let mut count = 0;
            while amount > 0 && count < addition.count {
                let d = damage(&result) - amount;
                result = with_damage(&data, &result, d);
                price += 1;
                amount = damage(&result).min(max_damage(&result) / 4);
                count += 1;
            }
            repair_count = count;
        } else {
            if !using_book && (!result.same_item(&addition) || !result.c.damageable()) {
                if let Some(a) = anvil_mut(m) {
                    a.repair_item_count_cost = 0;
                }
                fail(m);
                return;
            }
            if result.c.damageable() && !using_book {
                let remaining1 = max_damage(&input) - damage(&input);
                let remaining2 = max_damage(&addition) - damage(&addition);
                let additional = remaining2 + max_damage(&result) * 12 / 100;
                let remaining = remaining1 + additional;
                let result_damage = (max_damage(&result) - remaining).max(0);
                if result_damage < damage(&result) {
                    result = with_damage(&data, &result, result_damage);
                    price += 2;
                }
            }
            let mut any_compatible = false;
            let mut any_incompatible = false;
            // (In the order the server sent them; vanilla's goes by object hashes.)
            for (holder, level) in crafting_enchantments(&addition) {
                let current = enchantments
                    .iter()
                    .find(|(e, _)| *e == holder)
                    .map_or(0, |(_, l)| *l);
                let mut level = if current == level {
                    level + 1
                } else {
                    level.max(current)
                };
                let Some(def) = data.items.enchantments.get(holder) else {
                    continue;
                };
                let mut ok = def
                    .supported_items
                    .as_ref()
                    .is_some_and(|s| data.item_in(s, &input));
                if creative || is_book(&input) {
                    ok = true;
                }
                for &(other, _) in &enchantments {
                    if other != holder && !compatible(&data, holder, other) {
                        ok = false;
                        price += 1;
                    }
                }
                if !ok {
                    any_incompatible = true;
                } else {
                    any_compatible = true;
                    if level > def.max_level {
                        level = def.max_level;
                    }
                    // `ItemEnchantments.Mutable.set`: a level of 0 or less removes it.
                    match enchantments.iter_mut().find(|(e, _)| *e == holder) {
                        Some(e) if level > 0 => e.1 = level,
                        Some(_) => enchantments.retain(|(e, _)| *e != holder),
                        None if level > 0 => enchantments.push((holder, level)),
                        None => {}
                    }
                    let mut fee = def.anvil_cost;
                    if using_book {
                        fee = (fee / 2).max(1);
                    }
                    price += i64::from(fee) * i64::from(level);
                    if input.count > 1 {
                        price = 40;
                    }
                }
            }
            if any_incompatible && !any_compatible {
                if let Some(a) = anvil_mut(m) {
                    a.repair_item_count_cost = 0;
                }
                fail(m);
                return;
            }
        }
    }
    if let Some(a) = anvil_mut(m) {
        a.repair_item_count_cost = repair_count;
    }
    if let Some(name) = item_name.as_deref().filter(|n| !super::is_blank(Some(n))) {
        if name != data.hover_name(&input) {
            naming_cost = 1;
            price += naming_cost;
            result = super::set_component(
                &data.items,
                &result,
                ComponentKind::CustomName,
                Component::Other {
                    kind: ComponentKind::CustomName.id() as i32,
                    data: super::literal_text(name),
                },
                false,
            );
        }
    } else if input.c.custom_name.is_some() {
        naming_cost = 1;
        price += naming_cost;
        result = super::remove_component(&data.items, &result, ComponentKind::CustomName);
    }
    // (`price` is an int in vanilla; the fees can't make it overflow there either.)
    let price = price as i32 as i64;
    let final_price = if price <= 0 {
        0
    } else {
        (tax + price).clamp(0, i64::from(i32::MAX)) as i32
    };
    set_cost(m, final_price);
    if price <= 0 {
        result = Stack::empty();
    }
    if naming_cost == price && naming_cost > 0 {
        if m.data[0] >= 40 {
            set_cost(m, 39);
        }
        if let Some(a) = anvil_mut(m) {
            a.only_renaming = true;
        }
    }
    if m.data[0] >= 40 && !creative {
        result = Stack::empty();
    }
    if !result.is_empty() {
        let mut base = result.c.work.repair_cost;
        if base < addition.c.work.repair_cost {
            base = addition.c.work.repair_cost;
        }
        if naming_cost != price || naming_cost == 0 {
            base = increased_repair_cost(base);
        }
        let default = data.items.defaults(result.item).work.repair_cost == base;
        result = super::set_component(
            &data.items,
            &result,
            ComponentKind::RepairCost,
            Component::Other {
                kind: ComponentKind::RepairCost.id() as i32,
                data: super::varint_bytes(base),
            },
            default,
        );
        // `EnchantmentHelper.setEnchantments`: a book's stored ones, else its own.
        let (kind, value) = if is_book(&result) {
            (
                ComponentKind::StoredEnchantments,
                Component::StoredEnchantments(enchantments.clone()),
            )
        } else {
            (
                ComponentKind::Enchantments,
                Component::Enchantments(enchantments.clone()),
            )
        };
        result = super::set_component(&data.items, &result, kind, value, enchantments.is_empty());
    }
    m.own[2] = result;
    broadcast(m);
}

/// What the screen shows of the cost (`AnvilScreen.extractLabels`): the line's text (the
/// cost, or "Too Expensive!") and colour, if any.
pub fn cost_label(m: &Menu, creative: bool, who_may_pickup: bool) -> Option<(Option<i32>, u32)> {
    let cost = m.data.first().copied().unwrap_or(0);
    if cost <= 0 {
        return None;
    }
    if cost >= 40 && !creative {
        return Some((None, 0xFFFF_6060));
    }
    if m.own[2].is_empty() {
        return None;
    }
    let colour = if who_may_pickup {
        0xFF80_FF20
    } else {
        0xFFFF_6060
    };
    Some((Some(cost), colour))
}
