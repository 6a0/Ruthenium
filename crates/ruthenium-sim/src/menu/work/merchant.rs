//! `MerchantMenu`, `MerchantContainer`, `MerchantResultSlot` and `MerchantScreen` on the
//! client: the offers from `merchant_offers` (prices with demand and the special price,
//! out-of-stock trades), the sale the payment slots make (`updateSellItem`), taking it
//! (the payments shrink, the offer's uses grow), choosing a trade (`select_trade`, after
//! the payment slots are filled from the inventory as `tryMoveItems` does), the
//! villager's level and experience, and the screen's scrolling list.

use ruthenium_protocol::item::{Component, ComponentPatch, ItemStack};
use ruthenium_protocol::trade::MerchantOffer;

use super::{KindState, WorkData};
use crate::inventory::Inventory;
use crate::item::{Items, Stack};
use crate::menu::{Menu, Outcome, Who};
use ruthenium_javamath::mth;

/// `ItemCost`: an item, how many, the components it must have, and the stack it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Cost {
    pub item: i32,
    pub count: i32,
    pub components: Vec<(i32, Component)>,
    pub stack: Stack,
}

impl Cost {
    fn new(items: &Items, c: &ruthenium_protocol::trade::ItemCost) -> Cost {
        // `createStack`: the components as the stack's patch.
        let stack = items.stack(&ItemStack {
            item: c.item,
            count: c.count,
            patch: ComponentPatch {
                added: c.components.clone(),
                ..ComponentPatch::default()
            },
        });
        Cost {
            item: c.item,
            count: c.count,
            components: c.components.clone(),
            stack,
        }
    }

    /// `ItemCost.test`: the item, with each component exactly as asked.
    pub fn test(&self, s: &Stack) -> bool {
        !s.is_empty()
            && s.item == self.item
            && self
                .components
                .iter()
                .all(|(kind, value)| s.patch.added.iter().any(|(k, v)| k == kind && v == value))
    }
}

/// `MerchantOffer` on the client.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub cost_a: Cost,
    pub cost_b: Option<Cost>,
    pub result: Stack,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price_diff: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}

impl Offer {
    /// `createFromStream`: an exhausted offer's uses are its maximum.
    pub fn new(items: &Items, o: &MerchantOffer) -> Offer {
        Offer {
            cost_a: Cost::new(items, &o.cost_a),
            cost_b: o.cost_b.as_ref().map(|c| Cost::new(items, c)),
            result: items.stack(&o.result),
            uses: if o.out_of_stock { o.max_uses } else { o.uses },
            max_uses: o.max_uses,
            xp: o.xp,
            special_price_diff: o.special_price_diff,
            price_multiplier: o.price_multiplier,
            demand: o.demand,
        }
    }

    /// `getModifiedCostCount`.
    fn modified_count(&self, cost: &Cost) -> i32 {
        let base = cost.count;
        let product = base.wrapping_mul(self.demand) as f32 * self.price_multiplier;
        let demand_diff = mth::floor_f32(product).max(0);
        base.wrapping_add(demand_diff)
            .wrapping_add(self.special_price_diff)
            .clamp(1, cost.stack.max_stack_size().max(1))
    }

    /// `getCostA`: the price with demand and the special price.
    pub fn cost_a(&self) -> Stack {
        self.cost_a
            .stack
            .copy_with_count(self.modified_count(&self.cost_a))
    }

    /// `getBaseCostA`.
    pub fn base_cost_a(&self) -> &Stack {
        &self.cost_a.stack
    }

    /// `getCostB`.
    pub fn cost_b(&self) -> Stack {
        self.cost_b
            .as_ref()
            .map_or_else(Stack::empty, |c| c.stack.clone())
    }

    pub fn is_out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }

    /// `satisfiedBy`.
    pub fn satisfied_by(&self, a: &Stack, b: &Stack) -> bool {
        if !self.cost_a.test(a) || a.count < self.modified_count(&self.cost_a) {
            return false;
        }
        match &self.cost_b {
            None => b.is_empty(),
            Some(c) => c.test(b) && b.count >= c.count,
        }
    }

    /// `take`: the payments shrink by the price.
    fn take(&self, a: &mut Stack, b: &mut Stack) -> bool {
        if !self.satisfied_by(a, b) {
            return false;
        }
        a.shrink(self.cost_a().count);
        let cost_b = self.cost_b();
        if !cost_b.is_empty() {
            b.shrink(cost_b.count);
        }
        true
    }
}

/// The merchant's state (`ClientSideMerchant`, the menu's fields, the trade container's)
/// and its screen's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Merchant {
    pub offers: Vec<Offer>,
    pub xp: i32,
    pub level: i32,
    pub show_progress: bool,
    pub can_restock: bool,
    /// `MerchantContainer`'s.
    pub selection_hint: i32,
    pub active_offer: Option<usize>,
    pub future_xp: i32,
    // `MerchantScreen`'s.
    pub shop_item: i32,
    pub scroll_off: i32,
    pub dragging: bool,
    /// The trade button last pressed, which keeps the focus (and so its highlight).
    pub focused: Option<i32>,
}

pub(super) fn merchant(m: &Menu) -> Option<&Merchant> {
    match &m.work.as_ref()?.kind {
        KindState::Merchant(s) => Some(s),
        _ => None,
    }
}

pub(super) fn merchant_mut(m: &mut Menu) -> Option<&mut Merchant> {
    match &mut m.work.as_mut()?.kind {
        KindState::Merchant(s) => Some(s),
        _ => None,
    }
}

/// `handleMerchantOffers`.
pub fn set_offers(
    m: &mut Menu,
    data: &WorkData,
    offers: &[MerchantOffer],
    level: i32,
    xp: i32,
    show_progress: bool,
    can_restock: bool,
) {
    if let Some(s) = merchant_mut(m) {
        s.offers = offers.iter().map(|o| Offer::new(&data.items, o)).collect();
        s.active_offer = None;
        s.xp = xp;
        s.level = level;
        s.show_progress = show_progress;
        s.can_restock = can_restock;
    }
}

/// `MerchantOffers.getRecipeFor`.
fn recipe_for(offers: &[Offer], a: &Stack, b: &Stack, hint: i32) -> Option<usize> {
    if hint > 0 && (hint as usize) < offers.len() {
        return offers[hint as usize]
            .satisfied_by(a, b)
            .then_some(hint as usize);
    }
    offers.iter().position(|o| o.satisfied_by(a, b))
}

/// `MerchantContainer.updateSellItem`.
pub fn update_sell_item(m: &mut Menu) {
    let (p0, p1) = (m.own[0].clone(), m.own[1].clone());
    let (buy_a, buy_b) = if p0.is_empty() {
        (p1, Stack::empty())
    } else {
        (p0, p1)
    };
    let Some(s) = merchant_mut(m) else {
        return;
    };
    s.active_offer = None;
    let result = if buy_a.is_empty() {
        s.future_xp = 0;
        Stack::empty()
    } else if s.offers.is_empty() {
        return;
    } else {
        let mut offer = recipe_for(&s.offers, &buy_a, &buy_b, s.selection_hint);
        if offer.is_none_or(|o| s.offers[o].is_out_of_stock()) {
            s.active_offer = offer;
            offer = recipe_for(&s.offers, &buy_b, &buy_a, s.selection_hint);
        }
        match offer.filter(|&o| !s.offers[o].is_out_of_stock()) {
            Some(o) => {
                s.active_offer = Some(o);
                s.future_xp = s.offers[o].xp;
                s.offers[o].result.copy()
            }
            None => {
                s.future_xp = 0;
                Stack::empty()
            }
        }
    };
    m.own[2] = result;
}

/// The trade container's `setItem` (a payment's) and `setChanged` (any slot's): the sale
/// is worked out again.
pub(super) fn changed(m: &mut Menu, _index: usize) {
    update_sell_item(m);
}

/// `MerchantResultSlot.mayPickup`.
pub(super) fn may_pickup(m: &Menu) -> bool {
    let Some(s) = merchant(m) else {
        return false;
    };
    let Some(o) = s.active_offer.and_then(|o| s.offers.get(o)) else {
        return false;
    };
    let (a, b) = (&m.own[0], &m.own[1]);
    o.satisfied_by(a, b) || o.satisfied_by(b, a)
}

/// `MerchantResultSlot.onTake`: the price is paid from the slots, the offer used once
/// more (`ClientSideMerchant.notifyTrade`), and the villager's xp goes up.
pub(super) fn on_take(m: &mut Menu, taken: &mut Stack) {
    // `checkTakeAchievements`: `onCraftedBy`.
    if let Some(w) = m.work.as_ref() {
        let items = w.data.items.clone();
        super::crafted_by(&items, taken);
    }
    let Some(active) = merchant(m).and_then(|s| s.active_offer) else {
        return;
    };
    let Some(offer) = merchant(m).and_then(|s| s.offers.get(active)).cloned() else {
        return;
    };
    let (mut a, mut b) = (m.own[0].clone(), m.own[1].clone());
    if offer.take(&mut a, &mut b) || offer.take(&mut b, &mut a) {
        if let Some(s) = merchant_mut(m) {
            s.offers[active].uses += 1;
        }
        // (Both payments shrank in place before either `setItem` works out the sale.)
        let tidy = |s: Stack| if s.count <= 0 { Stack::empty() } else { s };
        m.own[0] = tidy(a);
        m.own[1] = tidy(b);
        update_sell_item(m);
        update_sell_item(m);
    }
    if let Some(s) = merchant_mut(m) {
        s.xp += offer.xp;
    }
}

/// `MerchantMenu.quickMoveStack`.
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
        m.move_item_stack_to(inv, &mut stack, 3, 39, true, who)
    } else if i != 0 && i != 1 {
        if (3..30).contains(&i) {
            m.move_item_stack_to(inv, &mut stack, 30, 39, false, who)
        } else if (30..39).contains(&i) {
            m.move_item_stack_to(inv, &mut stack, 3, 30, false, who)
        } else {
            true
        }
    } else {
        m.move_item_stack_to(inv, &mut stack, 3, 39, false, who)
    };
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    super::quick_move_tail(m, inv, i, stack, &clicked, who, out)
}

/// `setSelectionHint` and `tryMoveItems` (`MerchantScreen.postButtonClick`): the trade
/// chosen, the payments put back and the inventory's matching stacks brought in.
pub fn select(m: &mut Menu, inv: &mut Inventory, index: i32, who: &Who) {
    if let Some(s) = merchant_mut(m) {
        s.selection_hint = index;
    }
    update_sell_item(m);
    let Some(offer) = merchant(m)
        .and_then(|s| s.offers.get(usize::try_from(index).ok()?))
        .cloned()
    else {
        return;
    };
    for slot in [0, 1] {
        let mut old = m.own[slot].clone();
        if !old.is_empty() {
            if !m.move_item_stack_to(inv, &mut old, 3, 39, true, who) {
                return;
            }
            m.own[slot] = if old.count <= 0 { Stack::empty() } else { old };
            update_sell_item(m);
        }
    }
    if m.own[0].is_empty() && m.own[1].is_empty() {
        move_to_payment(m, inv, 0, &offer.cost_a);
        if let Some(b) = &offer.cost_b {
            move_to_payment(m, inv, 1, b);
        }
    }
}

/// `moveFromInventoryToPaymentSlot`.
fn move_to_payment(m: &mut Menu, inv: &mut Inventory, slot: usize, cost: &Cost) {
    for i in 3..39 {
        let mut here = m.get(inv, i);
        if here.is_empty() || !cost.test(&here) {
            continue;
        }
        let current = m.own[slot].clone();
        if current.is_empty() || here.same_item_same_components(&current) {
            let max = here.max_stack_size();
            let n = (max - current.count).min(here.count);
            let new = here.copy_with_count(current.count + n);
            here.shrink(n);
            m.own_or_inv_put(
                inv,
                i,
                if here.count <= 0 {
                    Stack::empty()
                } else {
                    here
                },
            );
            let full = new.count >= max;
            m.own[slot] = new;
            update_sell_item(m);
            if full {
                break;
            }
        }
    }
}

/// `VillagerData.getMinXpPerLevel`, `getMaxXpPerLevel`, `canLevelUp`.
pub const NEXT_LEVEL_XP: [i32; 5] = [0, 10, 70, 150, 250];

pub fn min_xp(level: i32) -> i32 {
    if can_level_up(level) {
        NEXT_LEVEL_XP[(level - 1) as usize]
    } else {
        0
    }
}

pub fn max_xp(level: i32) -> i32 {
    if can_level_up(level) {
        NEXT_LEVEL_XP[level as usize]
    } else {
        0
    }
}

pub fn can_level_up(level: i32) -> bool {
    (1..5).contains(&level)
}
