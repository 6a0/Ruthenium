//! `AbstractContainerScreen`'s handling of the mouse and keys: which slot is under
//! the pointer, what a press, a drag and a release click (dragging a stack over slots
//! spreads it: `QUICK_CRAFT`), double clicks gathering a stack (`PICKUP_ALL`), shift
//! moving stacks, the number keys and the off-hand key swapping with the hovered slot,
//! the drop key throwing from it, and the inventory key and Escape closing the screen.
//! The game thread runs it on the true menu as the frame's events come, as vanilla's
//! main thread does; drawing reads [`Screen::view`].

use super::{Container, Input, Kind, Menu, Who};
use crate::inventory::Inventory;
use crate::item::Stack;

/// The size of a menu's window in GUI units (`imageWidth`, `imageHeight`).
pub fn image_size(kind: Kind) -> (i32, i32) {
    match kind {
        Kind::Chest(rows) => (176, 114 + i32::from(rows) * 18),
        Kind::ShulkerBox => (176, 167),
        Kind::Hopper => (176, 133),
        _ => (176, 166),
    }
}

/// What of a key press or a mouse button a screen reads: the bindings on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Binds {
    pub escape: bool,
    pub inventory: bool,
    pub swap_offhand: bool,
    /// The hotbar bindings on it (slot 0 to 8).
    pub hotbar: [bool; 9],
    pub pick: bool,
    pub drop: bool,
}

/// A mouse button's event: SDL's numbering (1 left, 2 middle, 3 right), where the
/// pointer is in GUI units, and the modifiers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mouse {
    pub button: u8,
    pub x: f64,
    pub y: f64,
    pub shift: bool,
    pub binds: Binds,
}

/// A click the screen makes (`slotClicked`): slot, button and input.
pub type Click = (i32, i32, Input);

/// What a screen event asks of the menu around it.
pub trait Clicker {
    fn menu(&self) -> &Menu;
    fn inventory(&self) -> &Inventory;
    fn who(&self) -> Who<'_>;
    /// `MultiPlayerGameMode.handleContainerInput` for the screen's menu.
    fn click(&mut self, slot: i32, button: i32, input: Input);
}

/// `java.util.HashSet` of slots by their identity hash codes: the order a quick-craft
/// drag's slots are sent in (by bucket of the table, which only grows; within a
/// bucket by when they joined).
#[derive(Debug, Clone, Default)]
struct SlotSet {
    members: Vec<usize>,
    capacity: usize,
}

impl SlotSet {
    fn contains(&self, i: usize) -> bool {
        self.members.contains(&i)
    }

    fn insert(&mut self, i: usize) -> bool {
        if self.contains(i) {
            return false;
        }
        if self.capacity == 0 {
            self.capacity = 16;
        }
        self.members.push(i);
        while self.members.len() > self.capacity * 3 / 4 {
            self.capacity *= 2;
        }
        true
    }

    fn remove(&mut self, i: usize) {
        self.members.retain(|&m| m != i);
    }

    fn clear(&mut self) {
        self.members.clear();
    }

    fn len(&self) -> usize {
        self.members.len()
    }

    fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The order a `HashSet` iterates its members in.
    fn ordered(&self, hashes: &[u32]) -> Vec<usize> {
        let mask = self.capacity.max(16) - 1;
        let bucket = |i: usize| {
            let h = hashes.get(i).copied().unwrap_or(0);
            (h ^ (h >> 16)) as usize & mask
        };
        let mut out = self.members.clone();
        out.sort_by_key(|&i| bucket(i));
        out
    }
}

/// The screen's state between events (`AbstractContainerScreen`'s fields).
#[derive(Debug, Clone)]
pub struct Screen {
    pub kind: Kind,
    pub image_width: i32,
    pub image_height: i32,
    /// `leftPos`, `topPos`: the window's corner, from the GUI's size.
    pub left: i32,
    pub top: i32,
    pub hovered: Option<usize>,
    last_click_slot: Option<usize>,
    quick_craft_slots: SlotSet,
    is_quick_crafting: bool,
    quick_crafting_type: i32,
    quick_crafting_button: u8,
    skip_next_release: bool,
    quick_crafting_remainder: i32,
    doubleclick: bool,
    last_quick_moved: Stack,
}

/// What the drawing needs of the screen, as of the last event or frame.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub left: i32,
    pub top: i32,
    pub hovered: Option<usize>,
    /// A drag over more than one slot: its slots, its type and what is left on the
    /// cursor (`quickCraftingRemainder`).
    pub quick_craft: Option<(Vec<usize>, i32, i32)>,
}

/// `getContainerClickButton`: left is 0, right 1, any other its own number.
fn click_button(button: u8) -> i32 {
    match button {
        1 => 0,
        3 => 1,
        b => i32::from(b),
    }
}

impl Screen {
    /// A screen for `kind`'s menu on a GUI of `width` by `height` units (`init`).
    pub fn new(kind: Kind, width: i32, height: i32) -> Screen {
        let (image_width, image_height) = image_size(kind);
        let mut s = Screen {
            kind,
            image_width,
            image_height,
            left: 0,
            top: 0,
            hovered: None,
            last_click_slot: None,
            quick_craft_slots: SlotSet::default(),
            is_quick_crafting: false,
            quick_crafting_type: 0,
            quick_crafting_button: 0,
            skip_next_release: true,
            quick_crafting_remainder: 0,
            doubleclick: false,
            last_quick_moved: Stack::empty(),
        };
        s.resize(width, height);
        s
    }

    /// `init` again for a new GUI size.
    pub fn resize(&mut self, width: i32, height: i32) {
        self.left = (width - self.image_width) / 2;
        self.top = (height - self.image_height) / 2;
    }

    /// `isHovering`: within a slot's 16 units and one more around.
    fn is_hovering(&self, x: i32, y: i32, mx: f64, my: f64) -> bool {
        let (mx, my) = (mx - f64::from(self.left), my - f64::from(self.top));
        mx >= f64::from(x - 1)
            && mx < f64::from(x + 17)
            && my >= f64::from(y - 1)
            && my < f64::from(y + 17)
    }

    /// `getHoveredSlot`: the first slot under the pointer.
    pub fn slot_at(&self, menu: &Menu, mx: f64, my: f64) -> Option<usize> {
        menu.slots
            .iter()
            .position(|s| self.is_hovering(s.x, s.y, mx, my))
    }

    /// `hasClickedOutside`.
    fn clicked_outside(&self, mx: f64, my: f64) -> bool {
        mx < f64::from(self.left)
            || my < f64::from(self.top)
            || mx >= f64::from(self.left + self.image_width)
            || my >= f64::from(self.top + self.image_height)
    }

    /// `slotClicked`: a slot's own number wins over the one given.
    fn slot_clicked(
        c: &mut dyn Clicker,
        slot: Option<usize>,
        slot_id: i32,
        button: i32,
        input: Input,
    ) {
        let id = slot.map_or(slot_id, |s| s as i32);
        c.click(id, button, input);
    }

    /// The frame's drawing (`extractContents`, `extractSlot`): the slot under the
    /// pointer, and a drag's slots that no longer take the stack leave it.
    pub fn frame(&mut self, c: &dyn Clicker, mx: f64, my: f64) {
        // (Drawing takes the pointer in whole units.)
        self.hovered = self.slot_at(c.menu(), mx.floor(), my.floor());
        let (menu, inv) = (c.menu(), c.inventory());
        if self.is_quick_crafting && !menu.carried.is_empty() && self.quick_craft_slots.len() > 1 {
            let gone: Vec<usize> = self
                .quick_craft_slots
                .members
                .iter()
                .copied()
                .filter(|&i| {
                    !super::can_item_quick_replace(Some(menu.item(inv, i)), &menu.carried, true)
                })
                .collect();
            for i in gone {
                self.quick_craft_slots.remove(i);
                self.recalculate_remaining(c);
            }
        }
    }

    pub fn view(&self, menu: &Menu) -> View {
        View {
            left: self.left,
            top: self.top,
            hovered: self.hovered,
            quick_craft: (self.is_quick_crafting && self.quick_craft_slots.len() > 1).then(|| {
                (
                    self.quick_craft_slots.ordered(&menu.slot_hashes),
                    self.quick_crafting_type,
                    self.quick_crafting_remainder,
                )
            }),
        }
    }

    /// `recalculateQuickCraftRemaining`.
    fn recalculate_remaining(&mut self, c: &dyn Clicker) {
        let (menu, inv, who) = (c.menu(), c.inventory(), c.who());
        let carried = &menu.carried;
        if carried.is_empty() || !self.is_quick_crafting {
            return;
        }
        if self.quick_crafting_type == 2 {
            self.quick_crafting_remainder = carried.max_stack_size();
            return;
        }
        self.quick_crafting_remainder = carried.count;
        let n = self.quick_craft_slots.len();
        for &i in &self.quick_craft_slots.members {
            let here = menu.item(inv, i);
            let carry = if here.is_empty() { 0 } else { here.count };
            let max = carried
                .max_stack_size()
                .min(menu.max_stack_size(i, carried, &who));
            let count = (super::quick_craft_place_count(n, self.quick_crafting_type, carried)
                + carry)
                .min(max);
            self.quick_crafting_remainder -= count - carry;
        }
    }

    /// `mouseClicked`. `double_click`: the mouse handler's (the same button again on
    /// this screen within 250 ms).
    pub fn mouse_clicked(&mut self, c: &mut dyn Clicker, m: Mouse, double_click: bool) {
        let cloning = m.binds.pick && c.who().creative;
        let slot = self.slot_at(c.menu(), m.x, m.y);
        self.doubleclick = self.last_click_slot == slot && double_click;
        self.skip_next_release = false;
        if m.button != 1 && m.button != 3 && !cloning {
            // `checkHotbarMouseClicked`, on the slot under the pointer.
            self.hovered = slot;
            if let Some(h) = self.hovered.filter(|_| c.menu().carried.is_empty()) {
                if m.binds.swap_offhand {
                    Self::slot_clicked(c, Some(h), h as i32, 40, Input::Swap);
                } else {
                    for (i, _) in m.binds.hotbar.iter().enumerate().filter(|(_, b)| **b) {
                        Self::slot_clicked(c, Some(h), h as i32, i as i32, Input::Swap);
                    }
                }
            }
        } else {
            let outside = self.clicked_outside(m.x, m.y);
            let mut slot_id = slot.map_or(-1, |s| s as i32);
            if outside {
                slot_id = super::SLOT_OUTSIDE;
            }
            if slot_id != -1 && !self.is_quick_crafting {
                if c.menu().carried.is_empty() {
                    if cloning {
                        Self::slot_clicked(c, slot, slot_id, click_button(m.button), Input::Clone);
                    } else {
                        let quick = slot_id != super::SLOT_OUTSIDE && m.shift;
                        let mut input = Input::Pickup;
                        if quick {
                            self.last_quick_moved = slot
                                .map(|s| c.menu().item(c.inventory(), s).copy())
                                .unwrap_or_default();
                            input = Input::QuickMove;
                        } else if slot_id == super::SLOT_OUTSIDE {
                            input = Input::Throw;
                        }
                        Self::slot_clicked(c, slot, slot_id, click_button(m.button), input);
                    }
                    self.skip_next_release = true;
                } else {
                    self.is_quick_crafting = true;
                    self.quick_crafting_button = m.button;
                    self.quick_craft_slots.clear();
                    if m.button == 1 {
                        self.quick_crafting_type = 0;
                    } else if m.button == 3 {
                        self.quick_crafting_type = 1;
                    } else if cloning {
                        self.quick_crafting_type = 2;
                    }
                }
            }
        }
        self.last_click_slot = slot;
    }

    /// `mouseDragged` (once a frame while a button is held and the pointer moved).
    pub fn mouse_dragged(&mut self, c: &mut dyn Clicker, mx: f64, my: f64) {
        let Some(slot) = self.slot_at(c.menu(), mx, my) else {
            return;
        };
        let (menu, inv, who) = (c.menu(), c.inventory(), c.who());
        let carried = &menu.carried;
        let add = self.is_quick_crafting
            && !carried.is_empty()
            && (carried.count > self.quick_craft_slots.len() as i32
                || self.quick_crafting_type == 2)
            && super::can_item_quick_replace(Some(menu.item(inv, slot)), carried, true)
            && menu.may_place(slot, carried, &who);
        if add && self.quick_craft_slots.insert(slot) {
            self.recalculate_remaining(c);
        }
    }

    /// `mouseReleased`.
    pub fn mouse_released(&mut self, c: &mut dyn Clicker, m: Mouse) {
        let slot = self.slot_at(c.menu(), m.x, m.y);
        let outside = self.clicked_outside(m.x, m.y);
        let mut slot_id = slot.map_or(-1, |s| s as i32);
        if outside {
            slot_id = super::SLOT_OUTSIDE;
        }
        let pick_all = slot.filter(|&s| {
            self.doubleclick && m.button == 1 && c.menu().can_take_item_for_pick_all(s)
        });
        if let Some(s) = pick_all {
            if m.shift {
                if !self.last_quick_moved.is_empty() {
                    let container: Container = c.menu().slots[s].container;
                    for t in 0..c.menu().slots.len() {
                        let (menu, inv, who) = (c.menu(), c.inventory(), c.who());
                        if menu.may_pickup(inv, t, &who)
                            && !menu.item(inv, t).is_empty()
                            && menu.slots[t].container == container
                            && super::can_item_quick_replace(
                                Some(menu.item(inv, t)),
                                &self.last_quick_moved,
                                true,
                            )
                        {
                            Self::slot_clicked(
                                c,
                                Some(t),
                                t as i32,
                                click_button(m.button),
                                Input::QuickMove,
                            );
                        }
                    }
                }
            } else {
                Self::slot_clicked(c, slot, slot_id, click_button(m.button), Input::PickupAll);
            }
            self.doubleclick = false;
        } else {
            if self.is_quick_crafting && self.quick_crafting_button != m.button {
                self.is_quick_crafting = false;
                self.quick_craft_slots.clear();
                self.skip_next_release = true;
                return;
            }
            if self.skip_next_release {
                self.skip_next_release = false;
                return;
            }
            if self.is_quick_crafting && !self.quick_craft_slots.is_empty() {
                // `quickCraftToSlots`.
                let kind = self.quick_crafting_type;
                Self::slot_clicked(
                    c,
                    None,
                    super::SLOT_OUTSIDE,
                    super::quickcraft_mask(0, kind),
                    Input::QuickCraft,
                );
                for s in self.quick_craft_slots.ordered(&c.menu().slot_hashes) {
                    Self::slot_clicked(
                        c,
                        Some(s),
                        s as i32,
                        super::quickcraft_mask(1, kind),
                        Input::QuickCraft,
                    );
                }
                Self::slot_clicked(
                    c,
                    None,
                    super::SLOT_OUTSIDE,
                    super::quickcraft_mask(2, kind),
                    Input::QuickCraft,
                );
            } else if !c.menu().carried.is_empty() {
                if m.binds.pick {
                    Self::slot_clicked(c, slot, slot_id, click_button(m.button), Input::Clone);
                } else {
                    let quick = slot_id != super::SLOT_OUTSIDE && m.shift;
                    if quick {
                        self.last_quick_moved = slot
                            .map(|s| c.menu().item(c.inventory(), s).copy())
                            .unwrap_or_default();
                    }
                    let input = if quick {
                        Input::QuickMove
                    } else {
                        Input::Pickup
                    };
                    Self::slot_clicked(c, slot, slot_id, click_button(m.button), input);
                }
            }
        }
        self.is_quick_crafting = false;
    }

    /// `keyPressed` (a press or a held key's repeat), with the pointer at `(mx, my)`.
    /// Returns whether the screen closes (`onClose`: Escape, or the inventory key).
    pub fn key_pressed(
        &mut self,
        c: &mut dyn Clicker,
        binds: Binds,
        ctrl: bool,
        mx: f64,
        my: f64,
    ) -> bool {
        if binds.escape || binds.inventory {
            return true;
        }
        // (The slot the last frame drew as hovered: the pointer in whole units.)
        self.hovered = self.slot_at(c.menu(), mx.floor(), my.floor());
        // `checkHotbarKeyPressed` (its answer is not looked at).
        if let Some(h) = self.hovered.filter(|_| c.menu().carried.is_empty()) {
            if binds.swap_offhand {
                Self::slot_clicked(c, Some(h), h as i32, 40, Input::Swap);
            } else if let Some(i) = binds.hotbar.iter().position(|b| *b) {
                Self::slot_clicked(c, Some(h), h as i32, i as i32, Input::Swap);
            }
        }
        if let Some(h) = self
            .hovered
            .filter(|&h| !c.menu().item(c.inventory(), h).is_empty())
        {
            if binds.pick {
                Self::slot_clicked(c, Some(h), h as i32, 0, Input::Clone);
            } else if binds.drop {
                Self::slot_clicked(c, Some(h), h as i32, i32::from(ctrl), Input::Throw);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::quickcraft_mask;
    use super::*;
    use crate::item::{Items, RecipeSets};
    use ruthenium_data::v26_3::registries::Item;
    use ruthenium_prepare::exporters::ItemClassData;
    use std::collections::HashMap;

    fn items() -> Items {
        let mut classes = Vec::new();
        let mut reports = HashMap::new();
        for (name, max) in [("minecraft:stone", 64), ("minecraft:ender_pearl", 16)] {
            classes.push(ItemClassData {
                id: Item::from_name(name).unwrap().id(),
                key: name.to_owned(),
                classes: vec!["Item".to_owned()],
                flags: 0,
                block: None,
                fields: Vec::new(),
            });
            reports.insert(
                name.to_owned(),
                serde_json::json!({"components": {"minecraft:max_stack_size": max}}),
            );
        }
        Items::new(&classes, &reports)
    }

    struct Fake {
        menu: Menu,
        inv: Inventory,
        items: Items,
        recipes: RecipeSets,
        creative: bool,
        clicks: Vec<Click>,
    }

    impl Clicker for Fake {
        fn menu(&self) -> &Menu {
            &self.menu
        }
        fn inventory(&self) -> &Inventory {
            &self.inv
        }
        fn who(&self) -> Who<'_> {
            Who {
                items: &self.items,
                recipes: &self.recipes,
                creative: self.creative,
            }
        }
        fn click(&mut self, slot: i32, button: i32, input: Input) {
            self.clicks.push((slot, button, input));
            let who = Who {
                items: &self.items,
                recipes: &self.recipes,
                creative: self.creative,
            };
            self.menu.clicked(&mut self.inv, &who, slot, button, input);
        }
    }

    fn stone(f: &Fake, n: i32) -> Stack {
        f.items
            .new_stack(Item::from_name("minecraft:stone").unwrap().id() as i32, n)
    }

    /// A 3-row chest on a 400 by 300 GUI, with 10 stone in its slot 0 and 5 in 1.
    fn chest() -> (Screen, Fake) {
        let mut f = Fake {
            menu: Menu::new(Kind::Chest(3), 2),
            inv: Inventory::default(),
            items: items(),
            recipes: RecipeSets::new(),
            creative: false,
            clicks: Vec::new(),
        };
        f.menu.own[0] = stone(&f, 10);
        f.menu.own[1] = stone(&f, 5);
        (Screen::new(Kind::Chest(3), 400, 300), f)
    }

    /// The middle of slot `i`.
    fn at(s: &Screen, f: &Fake, i: usize) -> (f64, f64) {
        let slot = f.menu.slots[i];
        (
            f64::from(s.left + slot.x + 8),
            f64::from(s.top + slot.y + 8),
        )
    }

    fn mouse(button: u8, (x, y): (f64, f64), shift: bool) -> Mouse {
        Mouse {
            button,
            x,
            y,
            shift,
            binds: Binds {
                pick: button == 2,
                ..Binds::default()
            },
        }
    }

    fn click(s: &mut Screen, f: &mut Fake, button: u8, p: (f64, f64), shift: bool, double: bool) {
        s.mouse_clicked(f, mouse(button, p, shift), double);
        s.mouse_released(f, mouse(button, p, shift));
    }

    #[test]
    fn the_window_sits_in_the_middle() {
        let s = Screen::new(Kind::Chest(3), 400, 300);
        assert_eq!((s.image_width, s.image_height), (176, 168));
        assert_eq!((s.left, s.top), (112, 66));
        assert_eq!(image_size(Kind::Chest(6)), (176, 222));
        assert_eq!(image_size(Kind::Hopper), (176, 133));
        assert_eq!(image_size(Kind::ShulkerBox), (176, 167));
        assert_eq!(image_size(Kind::Inventory), (176, 166));
        // A slot is hovered from one unit before it to one after its 16.
        let (_, f) = chest();
        let slot = f.menu.slots[0];
        let (x, y) = (f64::from(s.left + slot.x), f64::from(s.top + slot.y));
        assert_eq!(s.slot_at(&f.menu, x - 1.0, y - 1.0), Some(0));
        assert_eq!(s.slot_at(&f.menu, x + 16.9, y + 16.9), Some(0));
        assert_eq!(s.slot_at(&f.menu, x - 1.1, y), None);
    }

    #[test]
    fn a_press_picks_up_and_its_release_is_skipped() {
        let (mut s, mut f) = chest();
        let p0 = at(&s, &f, 0);
        click(&mut s, &mut f, 1, p0, false, false);
        assert_eq!(f.clicks, [(0, 0, Input::Pickup)]);
        assert_eq!(f.menu.carried.count, 10);
        // With the stack on the cursor a press starts a drag; letting go over an empty
        // slot without dragging puts it down (one click, at the release).
        let p5 = at(&s, &f, 5);
        s.mouse_clicked(&mut f, mouse(1, p5, false), false);
        assert_eq!(f.clicks.len(), 1);
        s.mouse_released(&mut f, mouse(1, p5, false));
        assert_eq!(f.clicks[1], (5, 0, Input::Pickup));
        assert_eq!(f.menu.own[5].count, 10);
        // The right button takes half, rounded up.
        click(&mut s, &mut f, 3, p5, false, false);
        assert_eq!(f.clicks[2], (5, 1, Input::Pickup));
        assert_eq!(f.menu.carried.count, 5);
    }

    #[test]
    fn letting_go_between_slots_clicks_nothing_there() {
        let (mut s, mut f) = chest();
        f.menu.carried = stone(&f, 3);
        // Between the title and the first row: inside the window, over no slot.
        let p = (f64::from(s.left + 60), f64::from(s.top + 4));
        click(&mut s, &mut f, 1, p, false, false);
        assert_eq!(f.clicks, [(-1, 0, Input::Pickup)]);
        // Outside the window the stack is dropped (`-999`); with nothing carried, a
        // press there throws nothing but still says so.
        click(&mut s, &mut f, 1, (2.0, 2.0), false, false);
        assert_eq!(f.clicks[1], (-999, 0, Input::Pickup));
        assert!(f.menu.carried.is_empty());
        click(&mut s, &mut f, 3, (2.0, 2.0), false, false);
        assert_eq!(f.clicks[2], (-999, 1, Input::Throw));
    }

    #[test]
    fn dragging_spreads_the_stack() {
        let (mut s, mut f) = chest();
        f.menu.carried = stone(&f, 9);
        let (p3, p4, p5) = (at(&s, &f, 3), at(&s, &f, 4), at(&s, &f, 5));
        s.mouse_clicked(&mut f, mouse(1, p3, false), false);
        for p in [p3, p4, p5, p4] {
            s.mouse_dragged(&mut f, p.0, p.1);
        }
        let v = s.view(&f.menu);
        let (slots, kind, left) = v.quick_craft.unwrap();
        assert_eq!((slots.len(), kind, left), (3, 0, 0));
        s.mouse_released(&mut f, mouse(1, p4, false));
        assert_eq!(f.clicks.len(), 5);
        assert_eq!(
            f.clicks[0],
            (-999, quickcraft_mask(0, 0), Input::QuickCraft)
        );
        assert_eq!(
            f.clicks[4],
            (-999, quickcraft_mask(2, 0), Input::QuickCraft)
        );
        let mut middle: Vec<i32> = f.clicks[1..4].iter().map(|c| c.0).collect();
        middle.sort();
        assert_eq!(middle, [3, 4, 5]);
        assert!(f.clicks[1..4].iter().all(|c| c.1 == quickcraft_mask(1, 0)));
        assert_eq!([3, 4, 5].map(|i| f.menu.own[i].count), [3, 3, 3]);
        assert!(f.menu.carried.is_empty());
        // The right button drags one into each; a press of the other button ends it
        // with nothing sent.
        f.menu.carried = stone(&f, 2);
        s.mouse_clicked(&mut f, mouse(3, p3, false), false);
        s.mouse_dragged(&mut f, p3.0, p3.1);
        s.mouse_released(&mut f, mouse(1, p3, false));
        assert_eq!(f.clicks.len(), 5);
    }

    #[test]
    fn a_double_click_gathers_like_stacks() {
        let (mut s, mut f) = chest();
        let p1 = at(&s, &f, 1);
        click(&mut s, &mut f, 1, p1, false, false);
        s.mouse_clicked(&mut f, mouse(1, p1, false), true);
        s.mouse_released(&mut f, mouse(1, p1, false));
        assert_eq!(f.clicks, [(1, 0, Input::Pickup), (1, 0, Input::PickupAll)]);
        assert_eq!(f.menu.carried.count, 15);
        // Shift and a double click move every like stack of that container, when the
        // second press finds a stack there still (`lastQuickMoved` is taken again).
        let (mut s, mut f) = chest();
        let p0 = at(&s, &f, 0);
        click(&mut s, &mut f, 1, p0, true, false);
        assert_eq!(f.clicks, [(0, 0, Input::QuickMove)]);
        f.menu.own[0] = stone(&f, 2);
        s.mouse_clicked(&mut f, mouse(1, p0, true), true);
        s.mouse_released(&mut f, mouse(1, p0, true));
        assert_eq!(
            f.clicks[1..],
            [(0, 0, Input::QuickMove), (1, 0, Input::QuickMove)]
        );
        // An empty slot the second time: that press's move of nothing is all.
        let (mut s, mut f) = chest();
        click(&mut s, &mut f, 1, p0, true, false);
        s.mouse_clicked(&mut f, mouse(1, p0, true), true);
        s.mouse_released(&mut f, mouse(1, p0, true));
        assert_eq!(
            f.clicks,
            [(0, 0, Input::QuickMove), (0, 0, Input::QuickMove)]
        );
    }

    #[test]
    fn keys_swap_throw_and_close() {
        let (mut s, mut f) = chest();
        let p0 = at(&s, &f, 0);
        let mut hot = Binds::default();
        hot.hotbar[2] = true;
        assert!(!s.key_pressed(&mut f, hot, false, p0.0, p0.1));
        assert_eq!(f.clicks, [(0, 2, Input::Swap)]);
        assert_eq!(f.inv.items[2].count, 10);
        let off = Binds {
            swap_offhand: true,
            ..Binds::default()
        };
        let p1 = at(&s, &f, 1);
        s.key_pressed(&mut f, off, false, p1.0, p1.1);
        assert_eq!(f.clicks[1], (1, 40, Input::Swap));
        // The drop key only over a stack; with ctrl all of it.
        let drop = Binds {
            drop: true,
            ..Binds::default()
        };
        s.key_pressed(&mut f, drop, true, p1.0, p1.1);
        assert_eq!(f.clicks.len(), 2);
        f.menu.own[1] = stone(&f, 4);
        s.key_pressed(&mut f, drop, true, p1.0, p1.1);
        assert_eq!(f.clicks[2], (1, 1, Input::Throw));
        assert!(f.menu.own[1].is_empty());
        let esc = Binds {
            escape: true,
            ..Binds::default()
        };
        assert!(s.key_pressed(&mut f, esc, false, p1.0, p1.1));
        let inv = Binds {
            inventory: true,
            ..Binds::default()
        };
        assert!(s.key_pressed(&mut f, inv, false, p1.0, p1.1));
    }

    #[test]
    fn the_middle_button_clones_only_with_infinite_materials() {
        let (mut s, mut f) = chest();
        let p0 = at(&s, &f, 0);
        // In survival a press reads the mouse's hotbar bindings (none) and sends
        // nothing.
        click(&mut s, &mut f, 2, p0, false, false);
        assert_eq!(f.clicks, []);
        // ...but letting it go with a stack on the cursor sends a clone that does
        // nothing.
        f.menu.carried = stone(&f, 1);
        click(&mut s, &mut f, 2, p0, false, false);
        assert_eq!(f.clicks, [(0, 2, Input::Clone)]);
        assert_eq!(f.menu.carried.count, 1);
        let (mut s, mut f) = chest();
        f.creative = true;
        click(&mut s, &mut f, 2, p0, false, false);
        assert_eq!(f.clicks, [(0, 2, Input::Clone)]);
        assert_eq!(f.menu.carried.count, 64);
    }

    #[test]
    fn hash_sets_iterate_by_bucket() {
        let mut set = SlotSet::default();
        for i in [5, 1, 3] {
            set.insert(i);
        }
        // Buckets of 16: the hashes' low bits after the spread.
        let hashes = [0, 0x21, 0, 0x10_0002, 0, 0x7];
        assert_eq!(set.ordered(&hashes), [1, 3, 5]);
        // The table grows past 12 and never shrinks.
        for i in 0..13 {
            set.insert(10 + i);
        }
        set.clear();
        assert_eq!(set.capacity, 32);
    }
}
