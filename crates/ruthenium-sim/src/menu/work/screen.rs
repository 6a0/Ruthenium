//! The workstations' screens' own mouse, wheel and key handling, run before (or instead
//! of) `AbstractContainerScreen`'s: the anvil's name box, the stonecutter's and the
//! loom's buttons and scroll bars, the merchant's trade buttons and scroll bar, and the
//! crafter's slot switches (`CrafterScreen.slotClicked`). What they send goes into the
//! menu's outbox, ahead of any click the same event makes.

use super::edit_box::EditKey;
use super::{KindState, Station, crafter, loom, merchant, stonecutter};
use crate::menu::screen::{Clicker, Mouse, Screen};
use crate::menu::{Input, Menu};
use ruthenium_protocol::packets::play;

/// SDL's left button.
const LEFT: u8 = 1;

fn kind_mut(m: &mut Menu) -> Option<&mut KindState> {
    m.work.as_mut().map(|w| &mut w.kind)
}

fn post(m: &mut Menu, packet: Vec<u8>) {
    if let Some(w) = m.work.as_mut() {
        w.outbox.push(packet);
    }
}

/// `mouseClicked` of the station's screen, before the slots': whether one of its own
/// widgets took the press.
pub fn mouse_clicked(s: &Screen, c: &mut dyn Clicker, m: Mouse) -> bool {
    let Some(station) = c.menu().station() else {
        return false;
    };
    let (left, top) = (f64::from(s.left), f64::from(s.top));
    let mut taken = false;
    match station {
        // The name box takes a left press on it (`EditBox.onClick`; the caret isn't moved
        // to the pointer here).
        Station::Anvil => {
            taken = m.button == LEFT
                && m.x >= left + 62.0
                && m.x < left + 165.0
                && m.y >= top + 24.0
                && m.y < top + 36.0;
        }
        Station::Stonecutter => c.work(&mut |menu, _, _| {
            let Some(cut) = stonecutter::cutter(menu).cloned() else {
                return;
            };
            if !cut.display_recipes {
                return;
            }
            let (xo, yo) = (left + 52.0, top + 14.0);
            for index in cut.start_index..cut.start_index + 12 {
                let pos = index - cut.start_index;
                let xx = m.x - (xo + f64::from(pos % 4 * 16));
                let yy = m.y - (yo + f64::from(pos / 4 * 18));
                if xx >= 0.0
                    && yy >= 0.0
                    && xx < 16.0
                    && yy < 18.0
                    && stonecutter::click_menu_button(menu, index)
                {
                    let id = menu.id;
                    post(menu, play::container_button_click(id, index));
                    taken = true;
                    return;
                }
            }
            let (xo, yo) = (left + 119.0, top + 9.0);
            if m.x >= xo
                && m.x < xo + 12.0
                && m.y >= yo
                && m.y < yo + 54.0
                && let Some(st) = stonecutter::cutter_mut(menu)
            {
                st.scrolling = true;
            }
        }),
        Station::Loom => c.work(&mut |menu, _, _| {
            let Some(l) = loom::loom(menu).cloned() else {
                return;
            };
            if !l.display_patterns {
                return;
            }
            let (xo, yo) = (left + 60.0, top + 13.0);
            for row in 0..4 {
                for column in 0..4 {
                    let xx = m.x - (xo + f64::from(column * 14));
                    let yy = m.y - (yo + f64::from(row * 14));
                    let index = (row + l.start_row) * 4 + column;
                    if xx >= 0.0
                        && yy >= 0.0
                        && xx < 14.0
                        && yy < 14.0
                        && loom::click_menu_button(menu, index)
                    {
                        let id = menu.id;
                        post(menu, play::container_button_click(id, index));
                        taken = true;
                        return;
                    }
                }
            }
            let (xo, yo) = (left + 119.0, top + 9.0);
            if m.x >= xo
                && m.x < xo + 12.0
                && m.y >= yo
                && m.y < yo + 56.0
                && let Some(st) = loom::loom_mut(menu)
            {
                st.scrolling = true;
            }
        }),
        Station::Merchant => c.work(&mut |menu, inv, who| {
            let Some(t) = merchant::merchant(menu).cloned() else {
                return;
            };
            if t.offers.len() > 7
                && m.x > left + 94.0
                && m.x < left + 100.0
                && m.y > top + 18.0
                && m.y <= top + 158.0
                && let Some(t) = merchant::merchant_mut(menu)
            {
                t.dragging = true;
            }
            // The trade buttons (`Button.Plain`): one for each offer the list shows.
            if m.button != LEFT {
                return;
            }
            for i in 0..7 {
                let (x, y) = (left + 5.0, top + 18.0 + f64::from(i * 20));
                if (i as usize) < t.offers.len()
                    && m.x >= x
                    && m.y >= y
                    && m.x < x + 88.0
                    && m.y < y + 20.0
                {
                    // `postButtonClick`.
                    let shop = i + t.scroll_off;
                    if let Some(t) = merchant::merchant_mut(menu) {
                        t.shop_item = shop;
                        t.focused = Some(i);
                    }
                    merchant::select(menu, inv, shop, who);
                    post(menu, play::select_trade(shop));
                    taken = true;
                    return;
                }
            }
        }),
        _ => {}
    }
    taken
}

/// `mouseDragged` before the slots': whether a scroll bar took it.
pub fn mouse_dragged(s: &Screen, c: &mut dyn Clicker, my: f64) -> bool {
    let top = s.top as f32;
    let mut taken = false;
    c.work(&mut |menu, _, _| {
        taken = match kind_mut(menu) {
            Some(KindState::Stonecutter(st)) => {
                let count = st.recipes.len() as i32;
                let active = st.scrolling && st.display_recipes && count > 12;
                if active {
                    let (y0, y1) = (top + 14.0, top + 14.0 + 54.0);
                    st.scroll_offs = ((my as f32 - y0 - 7.5) / (y1 - y0 - 15.0)).clamp(0.0, 1.0);
                    let rows = (count + 3) / 4 - 3;
                    st.start_index = (f64::from(st.scroll_offs * rows as f32) + 0.5) as i32 * 4;
                }
                active
            }
            Some(KindState::Loom(l)) => {
                let offscreen = loom::total_rows(l) - 4;
                let active = l.scrolling && l.display_patterns && offscreen > 0;
                if active {
                    let (y0, y1) = (top + 13.0, top + 13.0 + 56.0);
                    l.scroll_offs = ((my as f32 - y0 - 7.5) / (y1 - y0 - 15.0)).clamp(0.0, 1.0);
                    l.start_row =
                        ((f64::from(l.scroll_offs * offscreen as f32) + 0.5) as i32).max(0);
                }
                active
            }
            Some(KindState::Merchant(t)) => {
                if t.dragging {
                    let max = t.offers.len() as i32 - 7;
                    let (y0, y1) = (top + 18.0, top + 18.0 + 139.0);
                    let scrolling = (my as f32 - y0 - 13.5) / (y1 - y0 - 27.0);
                    let scrolling = scrolling * max as f32 + 0.5;
                    t.scroll_off = (scrolling as i32).clamp(0, max.max(0));
                }
                t.dragging
            }
            _ => false,
        };
    });
    taken
}

/// `mouseReleased`: a scroll bar lets go.
pub fn mouse_released(c: &mut dyn Clicker) {
    c.work(&mut |menu, _, _| match kind_mut(menu) {
        Some(KindState::Stonecutter(st)) => st.scrolling = false,
        Some(KindState::Loom(l)) => l.scrolling = false,
        Some(KindState::Merchant(t)) => t.dragging = false,
        _ => {}
    });
}

/// `mouseScrolled`: the station's list scrolls (`scroll`: positive is up).
pub fn mouse_scrolled(c: &mut dyn Clicker, scroll: f64) {
    c.work(&mut |menu, _, _| match kind_mut(menu) {
        Some(KindState::Stonecutter(st)) => {
            let count = st.recipes.len() as i32;
            if st.display_recipes && count > 12 {
                let rows = (count + 3) / 4 - 3;
                let delta = scroll as f32 / rows as f32;
                st.scroll_offs = (st.scroll_offs - delta).clamp(0.0, 1.0);
                st.start_index = (f64::from(st.scroll_offs * rows as f32) + 0.5) as i32 * 4;
            }
        }
        Some(KindState::Loom(l)) => {
            let offscreen = loom::total_rows(l) - 4;
            if l.display_patterns && offscreen > 0 {
                let delta = scroll as f32 / offscreen as f32;
                l.scroll_offs = (l.scroll_offs - delta).clamp(0.0, 1.0);
                l.start_row = ((l.scroll_offs * offscreen as f32 + 0.5) as i32).max(0);
            }
        }
        Some(KindState::Merchant(t)) => {
            let n = t.offers.len() as i32;
            if n > 7 {
                t.scroll_off = ((f64::from(t.scroll_off) - scroll) as i32).clamp(0, n - 7);
            }
        }
        _ => {}
    });
}

/// `CrafterScreen.slotClicked`: an empty grid slot switched on or off first (and the
/// server told), before the click goes as usual.
pub fn slot_clicked(c: &mut dyn Clicker, slot: i32, button: i32, input: Input) {
    if c.menu().station() != Some(Station::Crafter) {
        return;
    }
    c.work(&mut |menu, inv, _| {
        if let Some((i, enabled)) = crafter::toggle(menu, inv, slot, button, input) {
            crafter::set_slot_state(menu, i, enabled);
            let id = menu.id;
            post(
                menu,
                play::container_slot_state_changed(i as i32, id, enabled),
            );
        }
    });
}

/// Typed text (`charTyped`): into the anvil's name box, a character at a time, while it
/// takes input.
pub fn text(c: &mut dyn Clicker, text: &str) {
    c.work(&mut |menu, _, _| {
        for ch in text.chars() {
            let Some(KindState::Anvil(a)) = kind_mut(menu) else {
                return;
            };
            if !a.name.editable || ch == '§' || ch < ' ' || ch == '\u{7F}' {
                continue;
            }
            let mut buf = [0u8; 4];
            if let Some(v) = a.name.insert_text(ch.encode_utf8(&mut buf)) {
                super::anvil::on_name_changed(menu, &v);
            }
        }
    });
}

/// A key for the anvil's name box (`EditBox.keyPressed`), ahead of the same key's
/// bindings. Whether the screen keeps the key from them (the box took it, or takes
/// input).
pub fn edit_key(c: &mut dyn Clicker, key: EditKey, shift: bool, ctrl: bool, paste: &str) -> bool {
    let mut swallow = false;
    c.work(&mut |menu, _, _| {
        let Some(KindState::Anvil(a)) = kind_mut(menu) else {
            return;
        };
        let (taken, changed) = a.name.key(key, shift, ctrl, paste);
        swallow = taken || a.name.editable;
        if let Some(v) = changed {
            super::anvil::on_name_changed(menu, &v);
        }
    });
    swallow
}

/// `containerTick`.
pub fn tick(m: &mut Menu) {
    if m.station() == Some(Station::Smithing) {
        super::smithing::tick(m);
    }
}

#[cfg(test)]
mod tests {
    use ruthenium_data::v26_3::packets::play as pl;
    use ruthenium_data::v26_3::registries::Item;
    use ruthenium_protocol::item::{ComponentPatch, ItemStack};
    use ruthenium_protocol::trade::{ItemCost, MerchantOffer};

    use super::super::tests::{World, world};
    use super::super::{PlayerFacts, Station};
    use super::*;
    use crate::inventory::Inventory;
    use crate::item::Stack;
    use crate::menu::screen::Binds;
    use crate::menu::{Kind, Who};

    struct NoRegistries;
    impl ruthenium_protocol::hashed::SyncedRegistries for NoRegistries {
        fn key(&self, _: &str, _: i32) -> Option<&str> {
            None
        }
    }

    /// A screen's clicker: what it sends, in order, as the session sends it.
    struct Fake {
        w: World,
        menu: Menu,
        inv: Inventory,
        sent: Vec<Vec<u8>>,
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
                items: &self.w.data.items,
                recipes: &self.w.recipes,
                creative: false,
            }
        }
        fn click(&mut self, slot: i32, button: i32, input: Input) {
            let who = Who {
                items: &self.w.data.items,
                recipes: &self.w.recipes,
                creative: false,
            };
            let (p, _) = self
                .menu
                .handle_container_input(&mut self.inv, &who, 7, slot, button, input, &NoRegistries)
                .unwrap();
            self.sent.extend(self.menu.take_outbox());
            self.sent.push(p);
        }
        fn work(&mut self, f: &mut dyn FnMut(&mut Menu, &mut Inventory, &Who)) {
            let who = Who {
                items: &self.w.data.items,
                recipes: &self.w.recipes,
                creative: false,
            };
            f(&mut self.menu, &mut self.inv, &who);
        }
    }

    impl Fake {
        fn new(station: Station) -> (Screen, Fake) {
            let w = world();
            let mut menu = Menu::new(Kind::Station(station), 7);
            let state = menu.work_mut().unwrap();
            state.data = w.data.clone();
            state.player = PlayerFacts {
                creative: false,
                xp_level: 30,
            };
            let s = Screen::new(Kind::Station(station), 400, 300);
            (
                s,
                Fake {
                    w,
                    menu,
                    inv: Inventory::default(),
                    sent: Vec::new(),
                },
            )
        }

        fn stack(&self, name: &str, count: i32) -> Stack {
            let id = Item::from_name(name).unwrap().id() as i32;
            self.w.data.items.new_stack(id, count)
        }

        /// What was sent since, the menu's own packets drained as the session drains them.
        fn take(&mut self) -> Vec<Vec<u8>> {
            let mut out = std::mem::take(&mut self.sent);
            out.extend(self.menu.take_outbox());
            out
        }
    }

    fn press(s: &mut Screen, f: &mut Fake, x: f64, y: f64) {
        let m = Mouse {
            button: LEFT,
            x: f64::from(s.left) + x,
            y: f64::from(s.top) + y,
            shift: false,
            binds: Binds::default(),
        };
        s.mouse_clicked(f, m, false);
        s.mouse_released(f, m);
    }

    #[test]
    fn the_stonecutter_chooses_and_scrolls_its_recipes() {
        let (mut s, mut f) = Fake::new(Station::Stonecutter);
        let stone = f.stack("minecraft:stone", 4);
        let mut inv = std::mem::take(&mut f.inv);
        f.menu.set_item(&mut inv, 0, 1, stone);
        f.inv = inv;
        let cut = super::stonecutter::cutter(&f.menu).unwrap().clone();
        assert_eq!(cut.recipes.len(), 16);
        assert!(cut.display_recipes);
        assert_eq!(f.menu.data[0], -1);
        // The first button: chosen, and the server told.
        press(&mut s, &mut f, 52.0 + 8.0, 14.0 + 9.0);
        assert_eq!(f.menu.data[0], 0);
        assert_eq!(
            f.take(),
            [ruthenium_protocol::packets::play::container_button_click(
                7, 0
            )]
        );
        // Again: nothing to choose, and the press goes on to the slots (none there).
        press(&mut s, &mut f, 52.0 + 8.0, 14.0 + 9.0);
        assert!(
            f.take()
                .iter()
                .all(|p| p[0] != pl::Serverbound::ContainerButtonClick.id() as u8)
        );
        // The wheel scrolls a row (16 recipes: one row off the screen).
        s.mouse_scrolled(&mut f, -1.0);
        assert_eq!(super::stonecutter::cutter(&f.menu).unwrap().start_index, 4);
        // A cell past the last recipe still counts as a press of its button.
        press(
            &mut s,
            &mut f,
            52.0 + 3.0 * 16.0 + 8.0,
            14.0 + 2.0 * 18.0 + 9.0,
        );
        assert_eq!(
            f.take(),
            [ruthenium_protocol::packets::play::container_button_click(
                7, 15
            )]
        );
        press(
            &mut s,
            &mut f,
            52.0 + 3.0 * 16.0 + 8.0,
            14.0 + 2.0 * 18.0 + 9.0,
        );
        let buttons: Vec<_> = f
            .take()
            .into_iter()
            .filter(|p| p[0] == pl::Serverbound::ContainerButtonClick.id() as u8)
            .collect();
        assert!(buttons.is_empty(), "the selected one again sends nothing");
        // Dragging the scroll bar to the top goes back to the first row.
        let m = Mouse {
            button: LEFT,
            x: f64::from(s.left) + 125.0,
            y: f64::from(s.top) + 12.0,
            shift: false,
            binds: Binds::default(),
        };
        s.mouse_clicked(&mut f, m, false);
        s.mouse_dragged(&mut f, m.x, f64::from(s.top));
        assert_eq!(super::stonecutter::cutter(&f.menu).unwrap().start_index, 0);
        s.mouse_released(&mut f, m);
        assert!(!super::stonecutter::cutter(&f.menu).unwrap().scrolling);
    }

    #[test]
    fn the_loom_lists_and_chooses_patterns() {
        let (mut s, mut f) = Fake::new(Station::Loom);
        let mut inv = std::mem::take(&mut f.inv);
        let (banner, dye) = (
            f.stack("minecraft:white_banner", 1),
            f.stack("minecraft:red_dye", 3),
        );
        f.menu.set_item(&mut inv, 0, 1, banner);
        f.menu.set_item(&mut inv, 1, 2, dye);
        f.inv = inv;
        let l = super::loom::loom(&f.menu).unwrap().clone();
        assert!(l.display_patterns && l.patterns.len() > 16);
        assert!(f.menu.own[3].is_empty());
        press(&mut s, &mut f, 60.0 + 14.0 + 7.0, 13.0 + 7.0);
        assert_eq!(f.menu.data[0], 1);
        assert_eq!(
            f.take(),
            [ruthenium_protocol::packets::play::container_button_click(
                7, 1
            )]
        );
        // The banner the client makes: one, with the pattern in red.
        let result = &f.menu.own[3];
        assert_eq!(result.count, 1);
        let layers = result.c.banner_patterns.as_ref().unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].1, 14);
    }

    #[test]
    fn a_trade_fills_the_payment_from_the_inventory() {
        let (mut s, mut f) = Fake::new(Station::Merchant);
        let emerald = Item::from_name("minecraft:emerald").unwrap().id() as i32;
        let diamond = Item::from_name("minecraft:diamond").unwrap().id() as i32;
        let offer = |cost: i32, count: i32| MerchantOffer {
            cost_a: ItemCost {
                item: cost,
                count,
                components: Vec::new(),
            },
            result: ItemStack {
                item: diamond,
                count: 1,
                patch: ComponentPatch::default(),
            },
            cost_b: None,
            out_of_stock: false,
            uses: 0,
            max_uses: 12,
            xp: 2,
            special_price_diff: 0,
            price_multiplier: 0.05,
            demand: 0,
        };
        let data = f.w.data.clone();
        super::merchant::set_offers(
            &mut f.menu,
            &data,
            &[offer(emerald, 5), offer(diamond, 1)],
            2,
            20,
            true,
            true,
        );
        f.inv.items[3] = f.stack("minecraft:emerald", 30);
        // The first trade's button: the payment comes in, the sale shows, the server told.
        press(&mut s, &mut f, 5.0 + 40.0, 18.0 + 10.0);
        assert_eq!(
            f.take(),
            [ruthenium_protocol::packets::play::select_trade(0)]
        );
        assert_eq!(f.menu.own[0].count, 30);
        assert!(f.inv.items[3].is_empty());
        assert_eq!(f.menu.own[2].item, diamond);
        let t = super::merchant::merchant(&f.menu).unwrap();
        assert_eq!((t.shop_item, t.focused, t.future_xp), (0, Some(0), 2));
        // Taking it pays five and works out the next sale.
        let mut inv = std::mem::take(&mut f.inv);
        let who = Who {
            items: &f.w.data.items,
            recipes: &f.w.recipes,
            creative: false,
        };
        f.menu.clicked(&mut inv, &who, 2, 0, Input::Pickup);
        assert_eq!(f.menu.carried.item, diamond);
        assert_eq!(f.menu.own[0].count, 25);
        assert_eq!(f.menu.own[2].item, diamond);
        let t = super::merchant::merchant(&f.menu).unwrap();
        assert_eq!((t.offers[0].uses, t.xp), (1, 22));
        // A price with demand: 5 + floor(5 * 10 * 0.05) = 7.
        let mut o = super::merchant::merchant(&f.menu).unwrap().offers[0].clone();
        o.demand = 10;
        assert_eq!(o.cost_a().count, 7);
    }

    #[test]
    fn a_crafter_slot_switches_before_the_click() {
        let (mut s, mut f) = Fake::new(Station::Crafter);
        let at = |i: usize, f: &Fake| {
            let sl = f.menu.slots[i];
            (f64::from(sl.x + 8), f64::from(sl.y + 8))
        };
        let (x, y) = at(4, &f);
        press(&mut s, &mut f, x, y);
        let sent = f.take();
        assert_eq!(
            sent[0],
            ruthenium_protocol::packets::play::container_slot_state_changed(4, 7, false)
        );
        assert_eq!(sent[1][0], pl::Serverbound::ContainerClick.id() as u8);
        assert!(super::crafter::is_disabled(&f.menu, 4));
        // A switched-off slot takes nothing; a press with a stack carried switches it on.
        let stone = f.stack("minecraft:stone", 1);
        f.menu.carried = stone.clone();
        let who = Who {
            items: &f.w.data.items,
            recipes: &f.w.recipes,
            creative: false,
        };
        assert!(!f.menu.may_place(4, &stone, &who));
        press(&mut s, &mut f, x, y);
        let sent = f.take();
        assert_eq!(
            sent[0],
            ruthenium_protocol::packets::play::container_slot_state_changed(4, 7, true)
        );
        assert!(!super::crafter::is_disabled(&f.menu, 4));
        // The server's powered flag.
        f.menu.set_data(9, 1);
        assert!(super::crafter::is_powered(&f.menu));
    }

    #[test]
    fn the_anvil_names_what_it_holds() {
        let (_s, mut f) = Fake::new(Station::Anvil);
        // Nothing in it: the box takes no text and no keys but its own.
        text(&mut f, "x");
        assert!(f.take().is_empty());
        assert!(edit_key(&mut f, EditKey::Backspace, false, false, ""));
        assert!(!edit_key(&mut f, EditKey::Other, false, false, ""));
        // A sword goes in: the box shows its name, and the server hears there is none.
        let sword = f.stack("minecraft:diamond_sword", 1);
        let mut inv = std::mem::take(&mut f.inv);
        f.menu.set_item(&mut inv, 0, 1, sword);
        f.inv = inv;
        assert_eq!(
            f.take(),
            [ruthenium_protocol::packets::play::rename_item("")]
        );
        let Some(KindState::Anvil(a)) = f.menu.work.as_ref().map(|w| &w.kind) else {
            panic!();
        };
        assert_eq!(a.name.value(), "item.minecraft.diamond_sword");
        assert!(a.name.editable);
        // Typing renames it, a character at a time; a new name costs a level.
        text(&mut f, "!!");
        assert_eq!(
            f.take(),
            [
                ruthenium_protocol::packets::play::rename_item("item.minecraft.diamond_sword!"),
                ruthenium_protocol::packets::play::rename_item("item.minecraft.diamond_sword!!"),
            ]
        );
        assert_eq!(f.menu.data[0], 1);
        assert!(f.menu.own[2].c.custom_name.is_some());
        // Any key while it takes input stays with it.
        assert!(edit_key(&mut f, EditKey::Other, false, false, ""));
        // Back to its own name: no name, nothing to pay.
        edit_key(&mut f, EditKey::Backspace, false, false, "");
        edit_key(&mut f, EditKey::Backspace, false, false, "");
        assert_eq!(
            f.take().last(),
            Some(&ruthenium_protocol::packets::play::rename_item(""))
        );
        assert!(f.menu.own[2].is_empty());
        assert_eq!(f.menu.data[0], 0);
    }
}
