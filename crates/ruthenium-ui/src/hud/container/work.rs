//! The workstations' screens (`AnvilScreen`, `SmithingScreen`, `StonecutterScreen`,
//! `LoomScreen`, `CartographyTableScreen`, `CrafterScreen`, `MerchantScreen`): what each
//! draws over and beside the slots, laid out as vanilla lays it out.
//!
//! Left out until their renderers exist: the smithing table's armour stand, the loom's
//! banner and its patterns' pictures, and the cartography table's maps.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use ruthenium_assets::Font;
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::text::Lang;
use ruthenium_sim::item::Stack;
use ruthenium_sim::menu::work::smithing::{Cycling, Template};
use ruthenium_sim::menu::work::{self, KindState, Station, merchant, stonecutter};
use ruthenium_sim::session::ContainerView;

use super::super::text::{HudMemory, Wrap, head_by_width, tail_by_width};
use super::super::{HudLayout, HudSprites, WHITE, text_width};
use super::{ContainerDraw, LABEL};
use crate::draw::SpriteInfo;

/// `AnvilScreen`'s cost colours and the ground under it (`1325400064`).
const COST_OK: u32 = 0xFF80_FF20;
const COST_BAD: u32 = 0xFFFF_6060;
const COST_GROUND: u32 = 0x4F00_0000;
/// `DyeColor.GRAY.getTextureDiffuseColor()`.
const GRAY_DYE: u32 = 0xFF47_4F52;

/// The workstations' sprites.
#[derive(Debug, Clone)]
pub(in crate::hud) struct WorkSprites {
    by_name: HashMap<&'static str, SpriteInfo>,
}

const SPRITES: &[&str] = &[
    "container/anvil/text_field",
    "container/anvil/text_field_disabled",
    "container/anvil/error",
    "container/smithing/error",
    "container/stonecutter/scroller",
    "container/stonecutter/scroller_disabled",
    "container/stonecutter/recipe_selected",
    "container/stonecutter/recipe_highlighted",
    "container/stonecutter/recipe",
    "container/slot/banner",
    "container/slot/dye",
    "container/slot/banner_pattern",
    "container/loom/scroller",
    "container/loom/scroller_disabled",
    "container/loom/pattern_selected",
    "container/loom/pattern_highlighted",
    "container/loom/pattern",
    "container/loom/error",
    "container/cartography_table/error",
    "container/cartography_table/scaled_map",
    "container/cartography_table/duplicated_map",
    "container/cartography_table/map",
    "container/cartography_table/locked",
    "container/crafter/disabled_slot",
    "container/crafter/powered_redstone",
    "container/crafter/unpowered_redstone",
    "container/villager/out_of_stock",
    "container/villager/experience_bar_background",
    "container/villager/experience_bar_current",
    "container/villager/experience_bar_result",
    "container/villager/scroller",
    "container/villager/scroller_disabled",
    "container/villager/trade_arrow_out_of_stock",
    "container/villager/trade_arrow",
    "container/villager/discount_strikethrough",
    "widget/button",
    "widget/button_highlighted",
];

impl WorkSprites {
    pub(in crate::hud) fn with(s: &impl Fn(&str) -> SpriteInfo) -> WorkSprites {
        let mut by_name = HashMap::new();
        let icons = work::smithing::TEMPLATE_ICONS
            .iter()
            .chain(&work::smithing::TRIM_BASE_ICONS)
            .chain(&work::smithing::TRIM_ADDITION_ICONS)
            .chain(&work::smithing::UPGRADE_BASE_ICONS)
            .chain(&work::smithing::UPGRADE_ADDITION_ICONS);
        for &n in SPRITES.iter().chain(icons) {
            by_name.insert(n, s(n));
        }
        WorkSprites { by_name }
    }

    fn get(&self, name: &str) -> &SpriteInfo {
        static NONE: LazyLock<SpriteInfo> = LazyLock::new(|| SpriteInfo {
            uv: [0.0; 4],
            width: 16,
            height: 16,
            scaling: ruthenium_assets::meta::GuiScaling::Stretch,
        });
        self.by_name.get(name).unwrap_or(&NONE)
    }
}

/// The window's picture of a station (`textures/gui/container/<name>.png`).
pub fn background_name(s: Station) -> &'static str {
    match s {
        Station::Anvil => "anvil",
        Station::Smithing => "smithing",
        Station::Stonecutter => "stonecutter",
        Station::Loom => "loom",
        Station::Cartography => "cartography_table",
        Station::Crafter => "crafter",
        Station::Merchant => "villager",
    }
}

/// Its size in texels (the villager's is twice as wide).
pub fn background_size(s: Station) -> [i32; 2] {
    match s {
        Station::Merchant => [512, 256],
        _ => [256, 256],
    }
}

/// Every station's picture.
pub const BACKGROUNDS: [&str; 7] = [
    "anvil",
    "smithing",
    "stonecutter",
    "loom",
    "cartography_table",
    "crafter",
    "villager",
];

/// Where a station's title goes (`titleLabelX`, `titleLabelY`), its text `width` wide;
/// `None` for the merchant, which draws its own.
pub fn title_pos(s: Station, image_width: i32, width: i32) -> Option<(i32, i32)> {
    Some(match s {
        Station::Anvil => (60, 6),
        Station::Smithing => (44, 15),
        Station::Stonecutter => (8, 5),
        Station::Loom | Station::Cartography => (8, 4),
        Station::Crafter => ((image_width - width) / 2, 6),
        Station::Merchant => return None,
    })
}

/// Texts made here, kept alive so the layout's memory (keyed by the text's address)
/// never mistakes one for another.
static TEXTS: LazyLock<Mutex<HashMap<String, Arc<Tag>>>> = LazyLock::new(Mutex::default);

fn text(key: String, make: impl FnOnce() -> Tag) -> Arc<Tag> {
    let mut t = TEXTS.lock().unwrap_or_else(|e| e.into_inner());
    if t.len() > 512 {
        t.clear();
    }
    t.entry(key).or_insert_with(|| Arc::new(make())).clone()
}

fn translated(key: &str, with: Vec<Tag>) -> Tag {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.into()));
    if !with.is_empty() {
        c.put("with", Tag::List(with));
    }
    Tag::Compound(c)
}

fn key_text(key: &str) -> Arc<Tag> {
    text(format!("key:{key}"), || translated(key, Vec::new()))
}

/// Where the pointer is, and what the draw needs, for one station's screen.
struct At<'a> {
    left: i32,
    top: i32,
    mouse: (i32, i32),
    v: &'a ContainerView,
    w: &'a work::State,
    station: Station,
}

impl At<'_> {
    fn hovering(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        let (mx, my) = (self.mouse.0 - self.left, self.mouse.1 - self.top);
        mx >= x - 1 && mx < x + w + 1 && my >= y - 1 && my < y + h + 1
    }

    fn has(&self, slot: usize) -> bool {
        self.v.stacks.get(slot).is_some_and(|s| !s.is_empty())
    }
}

impl HudLayout {
    #[allow(clippy::too_many_arguments)]
    fn sprite(
        &mut self,
        sprites: &HudSprites,
        name: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        colour: u32,
    ) {
        let s = sprites.container.work.get(name);
        self.under.blit(s, sprites.texture, x, y, w, h, colour);
    }

    #[allow(clippy::too_many_arguments)]
    fn sprite_part(
        &mut self,
        sprites: &HudSprites,
        name: &str,
        size: [i32; 2],
        uv: [i32; 2],
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) {
        let s = sprites.container.work.get(name);
        self.under
            .blit_part(s, sprites.texture, size, uv, x, y, w, h, WHITE);
    }

    /// A station's `extractBackground` past its window's picture, and the merchant's
    /// list and buttons (drawn before the slots).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn work_background(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        stacks: &mut Vec<Stack>,
        left: i32,
        top: i32,
    ) {
        let v = d.view;
        let (Some(w), ruthenium_sim::menu::Kind::Station(station)) = (v.work.as_deref(), v.kind)
        else {
            return;
        };
        let a = At {
            left,
            top,
            mouse: d.mouse,
            v,
            w,
            station,
        };
        match (&w.kind, station) {
            (KindState::Anvil(an), _) => {
                // `ItemCombinerScreen`: the error, then `AnvilScreen`'s text field.
                if (a.has(0) || a.has(1)) && !a.has(2) {
                    self.sprite(
                        sprites,
                        "container/anvil/error",
                        left + 99,
                        top + 45,
                        28,
                        21,
                        WHITE,
                    );
                }
                let field = if a.has(0) {
                    "container/anvil/text_field"
                } else {
                    "container/anvil/text_field_disabled"
                };
                self.sprite(sprites, field, left + 59, top + 20, 110, 16, WHITE);
                self.anvil_box(font, memory, an, left + 62, top + 24);
            }
            (KindState::Smithing(s), _) => {
                if v.data.first().is_some_and(|&e| e > 0) {
                    self.sprite(
                        sprites,
                        "container/smithing/error",
                        left + 65,
                        top + 46,
                        28,
                        21,
                        WHITE,
                    );
                }
                for (slot, c) in [(0, &s.template), (1, &s.base), (2, &s.addition)] {
                    if !a.has(slot) {
                        let (x, y, _) = v.slots[slot];
                        self.cycling(sprites, c, left + x, top + y, d.partial);
                    }
                }
            }
            (KindState::Stonecutter(st), _) => self.stonecutter(sprites, &a, st, stacks),
            (KindState::Loom(l), _) => self.loom(sprites, &a, l),
            (KindState::Cartography, _) => self.cartography(sprites, &a),
            (KindState::Merchant(t), _) => {
                self.merchant(sprites, font, lang, memory, &a, t, stacks)
            }
            (KindState::Crafter, _) => {}
        }
        let _ = (lang, a.station);
    }

    /// A slot's cycling empty icons (`CyclingSlotBackground.extractRenderState`).
    fn cycling(&mut self, sprites: &HudSprites, c: &Cycling, x: i32, y: i32, partial: f32) {
        for (icon, alpha) in c.shown(partial) {
            let colour = ((alpha.clamp(0.0, 1.0) * 255.0) as u32) << 24 | 0xFF_FFFF;
            self.sprite(sprites, icon, x, y, 16, 16, colour);
        }
    }

    /// The anvil's name box: no border, white, the caret blinking every 300 ms.
    fn anvil_box(
        &mut self,
        font: &Font,
        memory: &mut HudMemory,
        an: &work::anvil::Anvil,
        x: i32,
        y: i32,
    ) {
        let width = 103;
        let value = an.name.value();
        // (The box counts UTF-16 units; the drawing goes by bytes.)
        let byte_at = |units: usize| {
            let mut n = 0;
            for (i, c) in value.char_indices() {
                if n >= units {
                    return i;
                }
                n += c.len_utf16();
            }
            value.len()
        };
        let (cu, hu) = an.name.cursor();
        let (caret, anchor) = (byte_at(cu), byte_at(hu));
        let display = memory.edit_display(&value, caret, font, width);
        let shown = head_by_width(&value[display..], width, font);
        let shown_end = display + shown.len();
        let on_screen = (display..=shown_end).contains(&caret);
        let mut draw_x = x;
        if !shown.is_empty() {
            let half = if on_screen {
                &value[display..caret]
            } else {
                shown
            };
            if !half.is_empty() {
                self.text(format_args!("{half}"), draw_x, y, WHITE, true);
            }
            draw_x += text_width(font, half) + 1;
        }
        let insert =
            caret < value.len() || value.encode_utf16().count() >= work::anvil::MAX_NAME_LENGTH;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if caret > display { x + width } else { x };
        } else if insert {
            cursor_x -= 1;
            draw_x -= 1;
        }
        if !shown.is_empty() && on_screen && caret < shown_end {
            let rest = &value[caret..shown_end];
            self.text(format_args!("{rest}"), draw_x, y, WHITE, true);
        }
        let anchor = anchor.clamp(display, shown_end);
        if anchor != caret {
            let highlight_x = x + text_width(font, &value[display..anchor]);
            let (x0, x1) = (cursor_x.min(x + width), (highlight_x - 1).min(x + width));
            self.over
                .fill(x0.min(x1), y - 1, x0.max(x1), y + 10, 0x8000_00FF);
        }
        let blink = an
            .focused
            .is_none_or(|t| t.elapsed().as_millis() / 300 % 2 == 0);
        if blink && on_screen {
            if insert {
                self.over.fill(cursor_x, y - 1, cursor_x + 1, y + 9, WHITE);
            } else {
                self.text(format_args!("_"), cursor_x, y, WHITE, true);
            }
        }
        let _ = tail_by_width;
    }

    fn stonecutter(
        &mut self,
        sprites: &HudSprites,
        a: &At<'_>,
        st: &stonecutter::Stonecutter,
        stacks: &mut Vec<Stack>,
    ) {
        let (left, top) = (a.left, a.top);
        let count = st.recipes.len() as i32;
        let active = st.display_recipes && count > 12;
        let sy = (41.0 * st.scroll_offs) as i32;
        let scroller = if active {
            "container/stonecutter/scroller"
        } else {
            "container/stonecutter/scroller_disabled"
        };
        self.sprite(sprites, scroller, left + 119, top + 15 + sy, 12, 15, WHITE);
        if !st.display_recipes {
            return;
        }
        let (x, y) = (left + 52, top + 14);
        let selected = a.v.data.first().copied().unwrap_or(-1);
        let (mx, my) = a.mouse;
        for index in st.start_index..(st.start_index + 12).min(count) {
            let pos = index - st.start_index;
            let px = x + pos % 4 * 16;
            let py = y + pos / 4 * 18 + 2;
            let sprite = if index == selected {
                "container/stonecutter/recipe_selected"
            } else if mx >= px && my >= py && mx < px + 16 && my < py + 18 {
                "container/stonecutter/recipe_highlighted"
            } else {
                "container/stonecutter/recipe"
            };
            self.sprite(sprites, sprite, px, py - 1, 16, 18, WHITE);
        }
        for index in st.start_index..(st.start_index + 12).min(count) {
            let pos = index - st.start_index;
            let px = x + pos % 4 * 16;
            let py = y + pos / 4 * 18 + 2;
            let Some(e) = a.w.data.stonecutter.get(st.recipes[index as usize]) else {
                continue;
            };
            let s = stonecutter::display_stack(&a.w.data, &e.display);
            if !s.is_empty() {
                self.item_at(stacks, s, px, py);
            }
        }
    }

    fn item_at(&mut self, stacks: &mut Vec<Stack>, s: Stack, x: i32, y: i32) {
        self.items.push(super::super::HudItem {
            slot: stacks.len(),
            x,
            y,
            squeeze: None,
            z: 0.0,
        });
        stacks.push(s);
    }

    fn loom(&mut self, sprites: &HudSprites, a: &At<'_>, l: &work::loom::Loom) {
        let (left, top) = (a.left, a.top);
        for (slot, icon) in [
            (0, "container/slot/banner"),
            (1, "container/slot/dye"),
            (2, "container/slot/banner_pattern"),
        ] {
            if !a.has(slot) {
                let (x, y, _) = a.v.slots[slot];
                self.sprite(sprites, icon, left + x, top + y, 16, 16, WHITE);
            }
        }
        let offscreen = work::loom::total_rows(l) - 4;
        let active = l.display_patterns && l.patterns.len() > 16;
        let sy = (41.0 * l.scroll_offs) as i32;
        let scroller = if active {
            "container/loom/scroller"
        } else {
            "container/loom/scroller_disabled"
        };
        self.sprite(sprites, scroller, left + 119, top + 13 + sy, 12, 15, WHITE);
        let _ = offscreen;
        // (The banner it makes would be drawn at 141, 8: no banner renderer yet.)
        if l.has_max_patterns {
            let (x, y, _) = a.v.slots[3];
            self.sprite(
                sprites,
                "container/loom/error",
                left + x - 5,
                top + y - 5,
                26,
                26,
                WHITE,
            );
        }
        if !l.display_patterns {
            return;
        }
        let selected = a.v.data.first().copied().unwrap_or(-1);
        let (mx, my) = a.mouse;
        'rows: for row in 0..4 {
            for column in 0..4 {
                let index = (row + l.start_row) * 4 + column;
                if index as usize >= l.patterns.len() {
                    break 'rows;
                }
                let (px, py) = (left + 60 + column * 14, top + 13 + row * 14);
                let hovered = mx >= px && my >= py && mx < px + 14 && my < py + 14;
                let sprite = if index == selected {
                    "container/loom/pattern_selected"
                } else if hovered {
                    "container/loom/pattern_highlighted"
                } else {
                    "container/loom/pattern"
                };
                self.sprite(sprites, sprite, px, py, 14, 14, WHITE);
                // `extractBannerOnButton`: the grey banner; its pattern's picture is left
                // out (no banner pattern atlas yet).
                self.under.fill(px + 4, py + 2, px + 9, py + 12, GRAY_DYE);
            }
        }
    }

    fn cartography(&mut self, sprites: &HudSprites, a: &At<'_>) {
        let (xo, yo) = (a.left, a.top);
        let addition =
            a.v.stacks
                .get(1)
                .map_or("", |s| if s.is_empty() { "" } else { s.c.name });
        let duplication = addition == "minecraft:map";
        let scaling = addition == "minecraft:paper";
        let locking = addition == "minecraft:glass_pane";
        // (Without the map's data, as before the server sends it: not locked, no error,
        // and no map drawn: there is no map renderer yet.)
        let locked = false;
        if scaling && !locked {
            self.sprite(
                sprites,
                "container/cartography_table/scaled_map",
                xo + 67,
                yo + 13,
                66,
                66,
                WHITE,
            );
        } else if duplication {
            self.sprite(
                sprites,
                "container/cartography_table/duplicated_map",
                xo + 67 + 16,
                yo + 13,
                50,
                66,
                WHITE,
            );
            self.sprite(
                sprites,
                "container/cartography_table/duplicated_map",
                xo + 67,
                yo + 13 + 16,
                50,
                66,
                WHITE,
            );
        } else if locking {
            self.sprite(
                sprites,
                "container/cartography_table/map",
                xo + 67,
                yo + 13,
                66,
                66,
                WHITE,
            );
            self.sprite(
                sprites,
                "container/cartography_table/locked",
                xo + 118,
                yo + 60,
                10,
                14,
                WHITE,
            );
        } else {
            self.sprite(
                sprites,
                "container/cartography_table/map",
                xo + 67,
                yo + 13,
                66,
                66,
                WHITE,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn merchant(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        a: &At<'_>,
        t: &merchant::Merchant,
        stacks: &mut Vec<Stack>,
    ) {
        let (xo, yo) = (a.left, a.top);
        let offers = &t.offers;
        if offers.is_empty() {
            return;
        }
        let shop = t.shop_item;
        let Some(selected) = usize::try_from(shop).ok().and_then(|i| offers.get(i)) else {
            return;
        };
        if selected.is_out_of_stock() {
            self.sprite(
                sprites,
                "container/villager/out_of_stock",
                xo + 83 + 99,
                yo + 35,
                28,
                21,
                WHITE,
            );
        }
        // The trade buttons (`Button.Plain`, no text).
        let (mx, my) = a.mouse;
        for i in 0..7i32 {
            if i as usize >= offers.len() {
                break;
            }
            let (x, y) = (xo + 5, yo + 18 + i * 20);
            let hovered = mx >= x && my >= y && mx < x + 88 && my < y + 20;
            let sprite = if hovered || t.focused == Some(i) {
                "widget/button_highlighted"
            } else {
                "widget/button"
            };
            self.sprite(sprites, sprite, x, y, 88, 20, WHITE);
        }
        // `extractScroller`.
        let steps = offers.len() as i32 + 1 - 7;
        if steps > 1 {
            let left_over = 139 - (27 + (steps - 1) * 139 / steps);
            let step = 1 + left_over / steps + 139 / steps;
            let mut off = (t.scroll_off * step).min(113);
            if t.scroll_off == steps - 1 {
                off = 113;
            }
            self.sprite(
                sprites,
                "container/villager/scroller",
                xo + 94,
                yo + 18 + off,
                6,
                27,
                WHITE,
            );
        } else {
            self.sprite(
                sprites,
                "container/villager/scroller_disabled",
                xo + 94,
                yo + 18,
                6,
                27,
                WHITE,
            );
        }
        let can_scroll = offers.len() > 7;
        let mut offer_y = yo + 16 + 1;
        let sell_x = xo + 5 + 5;
        for (i, o) in offers.iter().enumerate() {
            let i = i as i32;
            if can_scroll && (i < t.scroll_off || i >= 7 + t.scroll_off) {
                continue;
            }
            let y = offer_y + 2;
            let (base, cost_a, cost_b) = (o.base_cost_a().clone(), o.cost_a(), o.cost_b());
            // `extractAndDecorateCostA`.
            self.item_at(stacks, cost_a.clone(), sell_x, y);
            if base.count == cost_a.count {
                self.decorations(font, &cost_a, sell_x, y, None, 0.0);
            } else {
                let one = |s: &Stack| (s.count == 1).then_some((1, WHITE));
                self.decorations(font, &base, sell_x, y, one(&base), 0.0);
                self.decorations(font, &cost_a, sell_x + 14, y, one(&cost_a), 0.0);
                let s = sprites
                    .container
                    .work
                    .get("container/villager/discount_strikethrough");
                self.over
                    .blit(s, sprites.texture, sell_x + 7, y + 12, 9, 2, WHITE);
            }
            if !cost_b.is_empty() {
                self.item_at(stacks, cost_b.clone(), xo + 5 + 35, y);
                self.decorations(font, &cost_b, xo + 5 + 35, y, None, 0.0);
            }
            let arrow = if o.is_out_of_stock() {
                "container/villager/trade_arrow_out_of_stock"
            } else {
                "container/villager/trade_arrow"
            };
            let s = sprites.container.work.get(arrow);
            self.over
                .blit(s, sprites.texture, xo + 5 + 35 + 20, y + 3, 10, 9, WHITE);
            self.item_at(stacks, o.result.clone(), xo + 5 + 68, y);
            self.decorations(font, &o.result, xo + 5 + 68, y, None, 0.0);
            offer_y += 20;
        }
        if t.show_progress {
            self.merchant_bar(sprites, t, xo, yo);
        }
        let _ = (lang, memory);
    }

    /// `extractProgressBar`.
    fn merchant_bar(&mut self, sprites: &HudSprites, t: &merchant::Merchant, xo: i32, yo: i32) {
        let level = t.level;
        if level >= 5 {
            return;
        }
        self.sprite(
            sprites,
            "container/villager/experience_bar_background",
            xo + 136,
            yo + 16,
            102,
            5,
            WHITE,
        );
        let min = merchant::min_xp(level);
        if t.xp >= min && merchant::can_level_up(level) {
            let multiplier = 102.0 / (merchant::max_xp(level) - min) as f32;
            let w = ((multiplier * (t.xp - min) as f32).floor() as i32).min(102);
            self.sprite_part(
                sprites,
                "container/villager/experience_bar_current",
                [102, 5],
                [0, 0],
                xo + 136,
                yo + 16,
                w,
                5,
            );
            if t.future_xp > 0 {
                let fw = ((t.future_xp as f32 * multiplier).floor() as i32).min(102 - w);
                self.sprite_part(
                    sprites,
                    "container/villager/experience_bar_result",
                    [102, 5],
                    [w, 0],
                    xo + 136 + w,
                    yo + 16,
                    fw,
                    5,
                );
            }
        }
    }

    /// A station's `extractLabels` beside the title's: the anvil's cost, the merchant's
    /// title, inventory and "Trades".
    #[allow(clippy::too_many_arguments)]
    pub(super) fn work_labels(
        &mut self,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        left: i32,
        top: i32,
    ) {
        let v = d.view;
        let (Some(w), ruthenium_sim::menu::Kind::Station(station)) = (v.work.as_deref(), v.kind)
        else {
            return;
        };
        match station {
            Station::Anvil => {
                let cost = v.data.first().copied().unwrap_or(0);
                if cost <= 0 {
                    return;
                }
                let creative = w.player.creative;
                let may_pickup = (creative || w.player.xp_level >= cost) && cost > 0;
                let (line, colour) = if cost >= 40 && !creative {
                    (key_text("container.repair.expensive"), COST_BAD)
                } else if v.stacks.get(2).is_none_or(Stack::is_empty) {
                    return;
                } else {
                    let t = text(format!("cost:{cost}"), || {
                        translated("container.repair.cost", vec![Tag::String(cost.to_string())])
                    });
                    (t, if may_pickup { COST_OK } else { COST_BAD })
                };
                let line = memory.lines(&line, Wrap::Single, font, lang)[0].clone();
                let iw = v.image_width;
                let tx = iw - 8 - line.width - 2;
                self.under.fill(
                    left + tx - 2,
                    top + 67,
                    left + iw - 8,
                    top + 79,
                    COST_GROUND,
                );
                self.styled(&line, [0, 0], 1, left + tx, top + 69, colour, true);
            }
            Station::Merchant => {
                let KindState::Merchant(t) = &w.kind else {
                    return;
                };
                let title = v
                    .title
                    .clone()
                    .unwrap_or_else(|| key_text("merchant.trades"));
                let iw = v.image_width;
                let level = t.level;
                let shown = if level > 0 && level <= 5 && t.show_progress {
                    let key = format!("title:{:p}:{level}", Arc::as_ptr(&title));
                    text(key, || {
                        translated(
                            "merchant.title",
                            vec![
                                (*title).clone(),
                                translated(&format!("merchant.level.{level}"), Vec::new()),
                            ],
                        )
                    })
                } else {
                    title.clone()
                };
                let line = memory.lines(&shown, Wrap::Single, font, lang)[0].clone();
                self.styled(
                    &line,
                    [0, 0],
                    1,
                    left + 49 + iw / 2 - line.width / 2,
                    top + 6,
                    LABEL,
                    false,
                );
                let inv = memory.lines(&key_text("container.inventory"), Wrap::Single, font, lang)
                    [0]
                .clone();
                self.styled(
                    &inv,
                    [0, 0],
                    1,
                    left + 107,
                    top + v.image_height - 94,
                    LABEL,
                    false,
                );
                let trades =
                    memory.lines(&key_text("merchant.trades"), Wrap::Single, font, lang)[0].clone();
                self.styled(
                    &trades,
                    [0, 0],
                    1,
                    left + 5 - trades.width / 2 + 48,
                    top + 6,
                    LABEL,
                    false,
                );
            }
            _ => {}
        }
    }

    /// The crafter's powered arrow (`extractRedstone`: from the screen's middle).
    pub(super) fn work_front(&mut self, sprites: &HudSprites, d: &ContainerDraw<'_>) {
        let v = d.view;
        if v.kind == ruthenium_sim::menu::Kind::Station(Station::Crafter) {
            let powered = v.data.get(9) == Some(&1);
            let sprite = if powered {
                "container/crafter/powered_redstone"
            } else {
                "container/crafter/unpowered_redstone"
            };
            let s = sprites.container.work.get(sprite);
            self.over.blit(
                s,
                sprites.texture,
                d.width / 2 + 9,
                d.height / 2 - 48,
                16,
                16,
                WHITE,
            );
        }
    }

    /// A crafter's switched-off slot, drawn instead of the slot (`extractDisabledSlot`).
    /// Whether the slot was one.
    pub(super) fn work_slot(
        &mut self,
        sprites: &HudSprites,
        v: &ContainerView,
        i: usize,
        x: i32,
        y: i32,
    ) -> bool {
        if v.kind != ruthenium_sim::menu::Kind::Station(Station::Crafter) || i >= 9 {
            return false;
        }
        if v.data.get(i) != Some(&1) {
            return false;
        }
        let s = sprites
            .container
            .work
            .get("container/crafter/disabled_slot");
        self.under
            .blit(s, sprites.texture, x - 1, y - 1, 18, 18, WHITE);
        true
    }

    /// A station's own tooltips (text, or an item's name), when the slots have none.
    /// Whether one was drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn work_tooltip(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        left: i32,
        top: i32,
        hovered: Option<usize>,
    ) -> bool {
        let v = d.view;
        let (Some(w), ruthenium_sim::menu::Kind::Station(station)) = (v.work.as_deref(), v.kind)
        else {
            return false;
        };
        let a = At {
            left,
            top,
            mouse: d.mouse,
            v,
            w,
            station,
        };
        let carried = !v.carried.is_empty();
        let (mx, my) = d.mouse;
        let lines = |memory: &mut HudMemory, t: &Arc<Tag>, wrap: Wrap| {
            memory.lines(t, wrap, font, lang).to_vec()
        };
        match &w.kind {
            KindState::Smithing(_) => {
                // `extractOnboardingTooltips`.
                let mut tip = None;
                if v.data.first().is_some_and(|&e| e > 0) && a.hovering(65, 46, 28, 21) {
                    tip = Some(key_text("container.upgrade.error_tooltip"));
                }
                if let Some(h) = hovered {
                    let template = v.stacks.first().cloned().unwrap_or_default();
                    let empty = v.stacks[h].is_empty();
                    if template.is_empty() {
                        if h == 0 {
                            tip = Some(key_text("container.upgrade.missing_template_tooltip"));
                        }
                    } else if let Some(t) = Template::of(&template).filter(|_| empty) {
                        if h == 1 {
                            tip = Some(key_text(t.base_description()));
                        } else if h == 2 {
                            tip = Some(key_text(t.addition_description()));
                        }
                    }
                }
                if let Some(t) = tip {
                    let l = lines(memory, &t, Wrap::Split(115));
                    self.text_tooltip(sprites, d, &l);
                    return true;
                }
            }
            KindState::Stonecutter(st) => {
                if st.display_recipes && !(hovered.is_some() && !carried) {
                    let (x0, y0) = (left + 52, top + 14);
                    let count = st.recipes.len() as i32;
                    for index in st.start_index..(st.start_index + 12).min(count) {
                        let pos = index - st.start_index;
                        let ix = x0 + pos % 4 * 16;
                        let iy = y0 + pos / 4 * 18 + 2;
                        if mx >= ix && mx < ix + 16 && my >= iy && my < iy + 18 {
                            let Some(e) = w.data.stonecutter.get(st.recipes[index as usize]) else {
                                continue;
                            };
                            let s = stonecutter::display_stack(&w.data, &e.display);
                            if !s.is_empty() {
                                self.item_tooltip(sprites, font, lang, memory, d, &s);
                                return true;
                            }
                        }
                    }
                }
            }
            KindState::Loom(l) => {
                if l.display_patterns {
                    let selected = v.data.first().copied().unwrap_or(-1);
                    for row in 0..4 {
                        for column in 0..4 {
                            let index = (row + l.start_row) * 4 + column;
                            let Some(&p) = l.patterns.get(index as usize) else {
                                continue;
                            };
                            let (px, py) = (left + 60 + column * 14, top + 13 + row * 14);
                            if index != selected
                                && mx >= px
                                && my >= py
                                && mx < px + 14
                                && my < py + 14
                            {
                                let dye = v.stacks.get(1).and_then(|s| s.c.work.dye).unwrap_or(0);
                                let colour =
                                    ruthenium_sim::item::DYE_NAMES[usize::from(dye.min(15))];
                                let Some(pattern) = w.data.banner_patterns.get(p as usize) else {
                                    continue;
                                };
                                let key = format!("{}.{colour}", pattern.translation_key);
                                let t = key_text(&key);
                                let l = lines(memory, &t, Wrap::Single);
                                self.text_tooltip(sprites, d, &l);
                                return true;
                            }
                        }
                    }
                }
            }
            KindState::Crafter => {
                if let Some(h) = hovered
                    && h < 9
                    && v.data.get(h) != Some(&1)
                    && !carried
                    && v.stacks[h].is_empty()
                {
                    let l = lines(memory, &key_text("gui.togglable_slot"), Wrap::Single);
                    self.text_tooltip(sprites, d, &l);
                    return true;
                }
            }
            KindState::Merchant(t) => {
                let offers = &t.offers;
                if let Some(sel) = usize::try_from(t.shop_item)
                    .ok()
                    .and_then(|i| offers.get(i))
                    && sel.is_out_of_stock()
                    && a.hovering(186, 35, 22, 21)
                    && t.can_restock
                {
                    let l = lines(memory, &key_text("merchant.deprecated"), Wrap::Single);
                    self.text_tooltip(sprites, d, &l);
                    return true;
                }
                // `TradeOfferButton.extractToolTip`.
                for i in 0..7i32 {
                    let (x, y) = (left + 5, top + 18 + i * 20);
                    if (i as usize) < offers.len()
                        && mx >= x
                        && my >= y
                        && mx < x + 88
                        && my < y + 20
                    {
                        let Some(o) = offers.get((i + t.scroll_off) as usize) else {
                            break;
                        };
                        let s = if mx < x + 20 {
                            o.cost_a()
                        } else if mx < x + 50 && mx > x + 30 {
                            o.cost_b()
                        } else if mx > x + 65 {
                            o.result.clone()
                        } else {
                            Stack::empty()
                        };
                        if !s.is_empty() {
                            self.item_tooltip(sprites, font, lang, memory, d, &s);
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn item_tooltip(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        s: &Stack,
    ) {
        let name = Arc::new(ruthenium_sim::item_name::styled_name(s));
        let line = memory.lines(&name, Wrap::Single, font, lang)[0].clone();
        self.tooltip(sprites, d, &line);
    }

    /// A text tooltip of some lines (`setTooltipForNextFrame` with plain lines: no room
    /// after the first).
    fn text_tooltip(
        &mut self,
        sprites: &HudSprites,
        d: &ContainerDraw<'_>,
        lines: &[Arc<super::super::TextLine>],
    ) {
        if lines.is_empty() {
            return;
        }
        let s = &sprites.container;
        let w = lines.iter().map(|l| l.width).max().unwrap_or(0);
        let h = -2 + 10 * lines.len() as i32;
        let (mut x, mut y) = (d.mouse.0 + 12, d.mouse.1 - 12);
        if x + w > d.width {
            x = (x - 24 - w).max(4);
        }
        if y + h + 3 > d.height {
            y = d.height - (h + 3);
        }
        let (x0, y0, pw, ph) = (x - 12, y - 12, w + 24, h + 24);
        self.over.blit(
            &s.tooltip_background,
            sprites.texture,
            x0,
            y0,
            pw,
            ph,
            WHITE,
        );
        self.over
            .blit(&s.tooltip_frame, sprites.texture, x0, y0, pw, ph, WHITE);
        for (i, l) in lines.iter().enumerate() {
            self.styled(l, [0, 0], 1, x, y + 10 * i as i32, WHITE, true);
        }
    }
}
