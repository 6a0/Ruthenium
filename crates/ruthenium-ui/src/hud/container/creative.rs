//! The creative inventory's own drawing (`CreativeModeInventoryScreen.extractBackground`
//! and `extractRenderState`): the tabs behind the window and the selected one in front,
//! each with its icon, the scroller, the search box (an unbordered `EditBox`), the tab's
//! title, and the tabs' and the destroy slot's tooltips. The rest is the container
//! screens' (`super::HudLayout::container`).

use std::sync::{Arc, LazyLock};

use ruthenium_assets::Font;
use ruthenium_protocol::text::Lang;
use ruthenium_sim::item::Stack;
use ruthenium_sim::menu::creative::{CreativeView, DESTROY_SLOT};

use super::super::text::{HudMemory, Wrap, head_by_width};
use super::super::{HudLayout, HudSprites, WHITE, text_width};
use super::{ContainerDraw, LABEL, translated};
use crate::draw::SpriteInfo;

/// `EditBox`'s selection (`textHighlight`, drawn as the chat's is).
const SELECTION: u32 = 0x8000_00FF;
/// The search box's place and width, from the window's corner.
const SEARCH_X: i32 = 82;
const SEARCH_Y: i32 = 6;
const SEARCH_WIDTH: i32 = 80;
const SEARCH_MAX: usize = 50;

static BIN: LazyLock<Arc<ruthenium_protocol::nbt::Tag>> =
    LazyLock::new(|| translated("inventory.binSlot"));

/// The creative inventory's sprites.
#[derive(Debug, Clone)]
pub(in crate::hud) struct CreativeSprites {
    /// Top unselected, top selected, bottom unselected, bottom selected: by column.
    tabs: [[SpriteInfo; 7]; 4],
    scroller: SpriteInfo,
    scroller_disabled: SpriteInfo,
}

impl CreativeSprites {
    pub(in crate::hud) fn with(s: &impl Fn(&str) -> SpriteInfo) -> CreativeSprites {
        let row = |name: &str| {
            std::array::from_fn(|i| {
                s(&format!(
                    "container/creative_inventory/tab_{name}_{}",
                    i + 1
                ))
            })
        };
        CreativeSprites {
            tabs: [
                row("top_unselected"),
                row("top_selected"),
                row("bottom_unselected"),
                row("bottom_selected"),
            ],
            scroller: s("container/creative_inventory/scroller"),
            scroller_disabled: s("container/creative_inventory/scroller_disabled"),
        }
    }
}

impl HudLayout {
    /// `extractTabButton`.
    fn creative_tab(
        &mut self,
        sprites: &HudSprites,
        cv: &CreativeView,
        i: usize,
        stacks: &mut Vec<Stack>,
        left: i32,
        top: i32,
    ) {
        let t = &cv.tabs[i];
        let selected = cv.selected == Some(i);
        let x = left + t.x;
        let y = if t.top {
            top - 28
        } else {
            top + ruthenium_sim::menu::creative::IMAGE_HEIGHT - 4
        };
        let set = usize::from(!t.top) * 2 + usize::from(selected);
        let column = usize::try_from(t.column).unwrap_or(0).min(6);
        let s = &sprites.container.creative;
        self.under
            .blit(&s.tabs[set][column], sprites.texture, x, y, 26, 32, WHITE);
        let icon_y = y + 16 - 8 + if t.top { 1 } else { -1 };
        self.item(stacks, t.icon.clone(), x + 13 - 8, icon_y, 0.0);
    }

    /// The tabs not selected (behind the window).
    pub(super) fn creative_behind(
        &mut self,
        sprites: &HudSprites,
        cv: &CreativeView,
        stacks: &mut Vec<Stack>,
        left: i32,
        top: i32,
    ) {
        for i in 0..cv.tabs.len() {
            if cv.selected != Some(i) {
                self.creative_tab(sprites, cv, i, stacks, left, top);
            }
        }
    }

    /// Over the window: the search box, the scroller, the selected tab and the title.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn creative_front(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        cv: &CreativeView,
        stacks: &mut Vec<Stack>,
        left: i32,
        top: i32,
    ) {
        if let Some(sb) = &cv.search {
            self.search_box(font, memory, sb, left + SEARCH_X, top + SEARCH_Y);
        }
        if let Some((scroll, enabled)) = cv.scroller {
            let s = &sprites.container.creative;
            let sprite = if enabled {
                &s.scroller
            } else {
                &s.scroller_disabled
            };
            let y = top + 18 + ((112 - 17) as f32 * scroll) as i32;
            self.under
                .blit(sprite, sprites.texture, left + 175, y, 12, 15, WHITE);
        }
        if let Some(i) = cv.selected {
            self.creative_tab(sprites, cv, i, stacks, left, top);
        }
        if let Some(title) = &cv.title {
            let line = memory.lines(title, Wrap::Single, font, lang)[0].clone();
            self.styled(&line, [0, 0], 1, left + 8, top + 6, LABEL, false);
        }
    }

    /// The search box (`EditBox.extractWidgetRenderState`, unbordered, white with a
    /// shadow): the text from where it is scrolled to, the caret and the selection.
    fn search_box(
        &mut self,
        font: &Font,
        memory: &mut HudMemory,
        sb: &ruthenium_sim::menu::creative::SearchBoxView,
        x: i32,
        text_y: i32,
    ) {
        let value = sb.text.as_str();
        // UTF-16 places to byte offsets.
        let byte = |units: usize| {
            let mut n = 0;
            for (i, c) in value.char_indices() {
                if n >= units {
                    return i;
                }
                n += c.len_utf16();
            }
            value.len()
        };
        let caret = byte(sb.cursor);
        let display = memory.edit_display(value, caret, font, SEARCH_WIDTH);
        let shown = head_by_width(&value[display..], SEARCH_WIDTH, font);
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
                self.text(format_args!("{half}"), draw_x, text_y, WHITE, true);
            }
            draw_x += text_width(font, half) + 1;
        }
        let insert =
            sb.cursor < value.encode_utf16().count() || value.encode_utf16().count() >= SEARCH_MAX;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if caret > display { x + SEARCH_WIDTH } else { x };
        } else if insert {
            cursor_x -= 1;
            draw_x -= 1;
        }
        if !shown.is_empty() && on_screen && caret < shown_end {
            let rest = &value[caret..shown_end];
            self.text(format_args!("{rest}"), draw_x, text_y, WHITE, true);
        }
        let anchor = byte(sb.highlight).clamp(display, shown_end);
        if anchor != caret {
            let highlight_x = x + text_width(font, &value[display..anchor]);
            let right = x + SEARCH_WIDTH;
            let (x0, x1) = (cursor_x.min(right), (highlight_x - 1).min(right));
            self.over.fill(
                x0.min(x1),
                text_y - 1,
                x0.max(x1),
                text_y + 1 + 9,
                SELECTION,
            );
        }
        let blink = sb.focused_at.elapsed().as_millis() / 300;
        if sb.focused && blink.is_multiple_of(2) && on_screen {
            if insert {
                self.over
                    .fill(cursor_x, text_y - 1, cursor_x + 1, text_y + 10, WHITE);
            } else {
                self.text(format_args!("_"), cursor_x, text_y, WHITE, true);
            }
        }
    }

    /// `checkTabHovering`'s tooltip (the first tab under the pointer), else the destroy
    /// slot's in the inventory tab.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn creative_tooltips(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        cv: &CreativeView,
        left: i32,
        top: i32,
    ) {
        let (mx, my) = (d.mouse.0 - left, d.mouse.1 - top);
        let hovering = |x: i32, y: i32, w: i32, h: i32| {
            mx >= x - 1 && mx < x + w + 1 && my >= y - 1 && my < y + h + 1
        };
        if let Some(t) = cv.tabs.iter().find(|t| hovering(t.x + 3, t.y + 3, 21, 27)) {
            let line = memory.lines(&t.title, Wrap::Single, font, lang)[0].clone();
            self.tooltip(sprites, d, &line);
            return;
        }
        if cv.inventory_tab
            && let Some(&(x, y, _)) = d.view.slots.get(DESTROY_SLOT)
            && hovering(x, y, 16, 16)
        {
            let line = memory.lines(&BIN, Wrap::Single, font, lang)[0].clone();
            self.tooltip(sprites, d, &line);
        }
    }
}
