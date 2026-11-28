//! The container screens over the HUD (`AbstractContainerScreen.extractRenderState`
//! and each kind's `extractBackground`), laid out as vanilla lays them out, in GUI
//! units: the darkened world, the window's picture, the furnace's fire and arrow, the
//! titles, the slots with their items (an empty armour or off-hand slot shows its
//! icon), the slot under the pointer lit, a drag's slots and what it would leave in
//! them, the stack on the cursor, and the hovered item's name in a tooltip.

use std::sync::{Arc, LazyLock};

use ruthenium_assets::Font;
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::text::Lang;
use ruthenium_sim::item::{EquipmentSlot, Stack};
use ruthenium_sim::menu::{self, Kind, Rule};
use ruthenium_sim::session::ContainerView;

use super::text::{HudMemory, Wrap};
use super::{
    BLACK, COOLDOWN_COLOUR, HudItem, HudLayout, HudSprites, WHITE, decimal, hsv_to_rgb, text_width,
};
use crate::draw::SpriteInfo;
use ruthenium_assets::meta::GuiScaling;
use ruthenium_javamath::mth;

mod creative;

/// The titles' colour (`-12566464`).
const LABEL: u32 = 0xFF40_4040;
/// `ChatFormatting.YELLOW`.
const YELLOW: u32 = 0xFFFF_FF55;
/// What lies over a slot a drag fills (`-2130706433`).
const DRAG: u32 = 0x80FF_FFFF;
/// The carried stack is drawn nearer than the slots' (`nextStratum`), in GUI units.
const CARRIED_Z: f32 = 32.0;

/// The container screens' sprites.
#[derive(Debug, Clone)]
pub(super) struct ContainerSprites {
    highlight_back: SpriteInfo,
    highlight_front: SpriteInfo,
    /// Head, chest, legs, feet, then the off hand's shield.
    icons: [SpriteInfo; 5],
    /// The furnace's, the blast furnace's and the smoker's fire and arrow.
    lit: [SpriteInfo; 3],
    burn: [SpriteInfo; 3],
    tooltip_background: SpriteInfo,
    tooltip_frame: SpriteInfo,
    creative: creative::CreativeSprites,
}

impl ContainerSprites {
    pub(super) fn with(s: &impl Fn(&str) -> SpriteInfo) -> ContainerSprites {
        let kinds = ["furnace", "blast_furnace", "smoker"];
        ContainerSprites {
            highlight_back: s("container/slot_highlight_back"),
            highlight_front: s("container/slot_highlight_front"),
            icons: ["helmet", "chestplate", "leggings", "boots", "shield"]
                .map(|n| s(&format!("container/slot/{n}"))),
            lit: kinds.map(|k| s(&format!("container/{k}/lit_progress"))),
            burn: kinds.map(|k| s(&format!("container/{k}/burn_progress"))),
            tooltip_background: s("tooltip/background"),
            tooltip_frame: s("tooltip/frame"),
            creative: creative::CreativeSprites::with(s),
        }
    }
}

/// What a frame's container screen is drawn from.
#[derive(Debug, Clone, Copy)]
pub struct ContainerDraw<'a> {
    pub view: &'a ContainerView,
    /// The GUI's size in its own units, and the pointer in whole units.
    pub width: i32,
    pub height: i32,
    pub mouse: (i32, i32),
    /// The window's picture (`textures/gui/container/...`), once it has arrived: the
    /// renderer's texture index.
    pub background: Option<u32>,
    /// How far the view's tick has come (the cooldowns).
    pub partial: f32,
}

/// The picture a kind's window is drawn from (`textures/gui/container/<name>.png`).
pub fn background_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Inventory => "inventory",
        Kind::Chest(_) => "generic_54",
        Kind::Dispenser => "dispenser",
        Kind::ShulkerBox => "shulker_box",
        Kind::Crafting => "crafting_table",
        Kind::Furnace => "furnace",
        Kind::BlastFurnace => "blast_furnace",
        Kind::Smoker => "smoker",
        Kind::Hopper => "hopper",
        // Pieces of the chest's window.
        Kind::Placeholder(_) => "generic_54",
        Kind::Creative => "creative_inventory/tab_items",
    }
}

/// The picture a view's window is drawn from: the creative inventory's selected tab's,
/// else its kind's.
pub fn view_background(view: &ContainerView) -> &str {
    view.creative
        .as_ref()
        .and_then(|c| {
            c.background
                .strip_prefix("textures/gui/container/")?
                .strip_suffix(".png")
        })
        .unwrap_or_else(|| background_name(view.kind))
}

/// Where a placeholder window's plain band ends: at its inventory part (96 units
/// tall), or at its bottom edge (7).
fn placeholder_body_end(inventory: bool, image_height: i32) -> i32 {
    image_height - if inventory { 96 } else { 7 }
}

/// Every picture a container screen may need.
pub const BACKGROUNDS: [&str; 12] = [
    "inventory",
    "generic_54",
    "dispenser",
    "shulker_box",
    "crafting_table",
    "furnace",
    "blast_furnace",
    "smoker",
    "hopper",
    "creative_inventory/tab_items",
    "creative_inventory/tab_item_search",
    "creative_inventory/tab_inventory",
];

fn translated(key: &str) -> Arc<Tag> {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.into()));
    Arc::new(Tag::Compound(c))
}

static CRAFTING: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("container.crafting"));
static INVENTORY: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("container.inventory"));
/// The lines a placeholder screen shows (Ruthenium's own, not a translation key), each
/// narrower than the window.
static NOT_YET: LazyLock<[Arc<Tag>; 2]> = LazyLock::new(|| {
    ["Ruthenium can't show", "this screen yet"].map(|s| Arc::new(Tag::String(s.into())))
});

/// A whole 256 by 256 picture (`blit` with a texture size).
const PICTURE: SpriteInfo = SpriteInfo {
    uv: [0.0, 0.0, 1.0, 1.0],
    width: 256,
    height: 256,
    scaling: GuiScaling::Stretch,
};

impl HudLayout {
    /// `fillGradient` from top to bottom, one quad for each step of the colour.
    fn gradient(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, top: u32, bottom: u32) {
        let h = y1 - y0;
        if h <= 0 {
            return;
        }
        let channel = |c: u32, s: u32| (c >> s & 0xFF) as f32;
        let at = |y: i32| {
            let f = (y as f32 + 0.5) / h as f32;
            [24, 16, 8, 0].iter().fold(0u32, |acc, &s| {
                let v = channel(top, s) + (channel(bottom, s) - channel(top, s)) * f;
                acc | (v.round() as u32) << s
            })
        };
        let mut start = 0;
        let mut colour = at(0);
        for y in 1..=h {
            let c = if y < h { at(y) } else { !colour };
            if c != colour {
                self.under.fill(x0, y0 + start, x1, y0 + y, colour);
                start = y;
                colour = c;
            }
        }
    }

    /// `itemDecorations`: how worn it is, how long until it can be used again, and how
    /// many there are (or `count`, in its colour).
    fn decorations(
        &mut self,
        font: &Font,
        stack: &Stack,
        x: i32,
        y: i32,
        count: Option<(i32, u32)>,
        cooldown: f32,
    ) {
        if stack.is_empty() {
            return;
        }
        let c = &stack.c;
        let damage = c.damage.unwrap_or(0);
        if c.damageable() && damage > 0 {
            let max = c.max_damage.unwrap_or(0);
            let width = mth::floor_f32(13.0 - damage as f32 * 13.0 / max as f32 + 0.5).clamp(0, 13);
            let left = ((max as f32 - damage as f32) / max as f32).max(0.0);
            let colour = BLACK | hsv_to_rgb(left / 3.0, 1.0, 1.0);
            self.over.fill(x + 2, y + 13, x + 15, y + 15, BLACK);
            self.over.fill(x + 2, y + 13, x + 2 + width, y + 14, colour);
        }
        if cooldown > 0.0 {
            let top = y + mth::floor_f32(16.0 * (1.0 - cooldown));
            let bottom = top + (16.0 * cooldown).ceil() as i32;
            self.over.fill(x, top, x + 16, bottom, COOLDOWN_COLOUR);
        }
        if stack.count != 1 || count.is_some() {
            let (n, colour) = count.unwrap_or((stack.count, WHITE));
            let mut digits = [0u8; 12];
            let text = decimal(n, &mut digits);
            let width = text_width(font, text);
            self.text(
                format_args!("{text}"),
                x + 19 - 2 - width,
                y + 6 + 3,
                colour,
                true,
            );
        }
    }

    fn item(&mut self, stacks: &mut Vec<Stack>, stack: Stack, x: i32, y: i32, z: f32) {
        self.items.push(HudItem {
            slot: stacks.len(),
            x,
            y,
            squeeze: None,
            z,
        });
        stacks.push(stack);
    }

    /// Lays out a container screen over the HUD. `stacks` gets what its items show
    /// (`HudItem::slot` numbers them).
    pub fn container(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
        stacks: &mut Vec<Stack>,
    ) {
        self.clear();
        stacks.clear();
        memory.begin();
        let s = &sprites.container;
        let v = d.view;
        let (iw, ih) = (v.image_width, v.image_height);
        let (left, top) = ((d.width - iw) / 2, (d.height - ih) / 2);
        // `extractTransparentBackground`.
        self.gradient(0, 0, d.width, d.height, 0xC010_1010, 0xD010_1010);
        if let Some(cv) = &v.creative {
            self.creative_behind(sprites, cv, stacks, left, top);
        }
        if let Some(t) = d.background {
            match v.kind {
                Kind::Chest(rows) => {
                    let rows = i32::from(rows) * 18 + 17;
                    self.under.blit_part(
                        &PICTURE,
                        t,
                        [256, 256],
                        [0, 0],
                        left,
                        top,
                        iw,
                        rows,
                        WHITE,
                    );
                    self.under.blit_part(
                        &PICTURE,
                        t,
                        [256, 256],
                        [0, 126],
                        left,
                        top + rows,
                        iw,
                        96,
                        WHITE,
                    );
                }
                Kind::Placeholder(p) => {
                    // The chest window's top edge and title band, its plain band (rows 4
                    // to 16) as often as it takes, then its inventory part or its bottom
                    // edge.
                    let mut piece = |uv: [i32; 2], y: i32, h: i32| {
                        self.under.blit_part(
                            &PICTURE,
                            t,
                            [256, 256],
                            uv,
                            left,
                            top + y,
                            iw,
                            h,
                            WHITE,
                        );
                    };
                    piece([0, 0], 0, 17);
                    let end = placeholder_body_end(p.inventory, ih);
                    let mut y = 17;
                    while y < end {
                        let h = (end - y).min(13);
                        piece([0, 4], y, h);
                        y += h;
                    }
                    if p.inventory {
                        piece([0, 126], end, 96);
                    } else {
                        piece([0, 215], end, 7);
                    }
                }
                _ => {
                    self.under
                        .blit_part(&PICTURE, t, [256, 256], [0, 0], left, top, iw, ih, WHITE)
                }
            }
        }
        if let Kind::Placeholder(p) = v.kind {
            let middle = (17 + placeholder_body_end(p.inventory, ih)) / 2;
            for (i, text) in NOT_YET.iter().enumerate() {
                let line = memory.lines(text, Wrap::Single, font, lang)[0].clone();
                let x = left + (iw - line.width) / 2;
                let y = top + middle - 10 + 11 * i as i32;
                self.styled(&line, [0, 0], 1, x, y, LABEL, false);
            }
        }
        // `AbstractFurnaceScreen.extractBackground`: the fire while it burns, the arrow
        // as far as the cooking has come.
        let furnace = match v.kind {
            Kind::Furnace => Some(0),
            Kind::BlastFurnace => Some(1),
            Kind::Smoker => Some(2),
            _ => None,
        };
        if let Some(k) = furnace
            && let [lit, duration, progress, total, ..] = v.data[..]
        {
            let t = sprites.texture;
            if lit > 0 {
                let duration = if duration == 0 { 200 } else { duration };
                let p = (lit as f32 / duration as f32).clamp(0.0, 1.0);
                let h = (p * 13.0).ceil() as i32 + 1;
                self.under.blit_part(
                    &s.lit[k],
                    t,
                    [14, 14],
                    [0, 14 - h],
                    left + 56,
                    top + 36 + 14 - h,
                    14,
                    h,
                    WHITE,
                );
            }
            let p = if total != 0 && progress != 0 {
                (progress as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let w = (p * 24.0).ceil() as i32;
            self.under.blit_part(
                &s.burn[k],
                t,
                [24, 16],
                [0, 0],
                left + 79,
                top + 34,
                w,
                16,
                WHITE,
            );
        }
        if let Some(cv) = &v.creative {
            self.creative_front(sprites, font, lang, memory, cv, stacks, left, top);
        }
        // `extractLabels`: the title, and the inventory's name over its rows (not in
        // the inventory screen itself; the creative screen's are above).
        let title = v.title.clone().unwrap_or_else(|| CRAFTING.clone());
        let title = memory.lines(&title, Wrap::Single, font, lang)[0].clone();
        let title_x = match v.kind {
            Kind::Inventory => 97,
            Kind::Crafting => 29,
            Kind::Dispenser | Kind::Furnace | Kind::BlastFurnace | Kind::Smoker => {
                (iw - title.width) / 2
            }
            _ => 8,
        };
        if v.creative.is_none() {
            self.styled(&title, [0, 0], 1, left + title_x, top + 6, LABEL, false);
        }
        let shows_inventory = match v.kind {
            Kind::Inventory | Kind::Creative => false,
            Kind::Placeholder(p) => p.inventory,
            _ => true,
        };
        if shows_inventory {
            let inventory = memory.lines(&INVENTORY, Wrap::Single, font, lang)[0].clone();
            self.styled(&inventory, [0, 0], 1, left + 8, top + ih - 94, LABEL, false);
        }
        // `getHoveredSlot`: the first slot within a unit of the pointer.
        let (mx, my) = (d.mouse.0 - left, d.mouse.1 - top);
        let hovered = v
            .slots
            .iter()
            .position(|&(x, y, _)| mx >= x - 1 && mx < x + 17 && my >= y - 1 && my < y + 17);
        if let Some(h) = hovered {
            let (x, y, _) = v.slots[h];
            self.under.blit(
                &s.highlight_back,
                sprites.texture,
                left + x - 4,
                top + y - 4,
                24,
                24,
                WHITE,
            );
        }
        let carried = &v.carried;
        let drag = v
            .screen
            .quick_craft
            .as_ref()
            .filter(|_| !carried.is_empty());
        for (i, &(x, y, rule)) in v.slots.iter().enumerate() {
            // A placeholder's own slots aren't drawn, nor the creative inventory tab's
            // crafting slots (at -2000).
            if x == menu::HIDDEN || x <= -2000 {
                continue;
            }
            let (x, y) = (left + x, top + y);
            let mut stack = v.stacks[i].clone();
            let mut count = None;
            let mut dragged = false;
            if let Some((slots, kind, _)) = drag
                && slots.contains(&i)
                && menu::can_item_quick_replace(Some(&stack), carried, true)
            {
                dragged = true;
                let max = carried
                    .max_stack_size()
                    .min(menu::rule_max_stack_size(rule, carried));
                let carry = if stack.is_empty() { 0 } else { stack.count };
                let mut n = menu::quick_craft_place_count(slots.len(), *kind, carried) + carry;
                if n > max {
                    n = max;
                    count = Some((max, YELLOW));
                }
                stack = carried.copy_with_count(n);
            }
            if stack.is_empty() {
                let icon = match rule {
                    Rule::Armor(EquipmentSlot::Head) => Some(0),
                    Rule::Armor(EquipmentSlot::Chest) => Some(1),
                    Rule::Armor(EquipmentSlot::Legs) => Some(2),
                    Rule::Armor(EquipmentSlot::Feet) => Some(3),
                    Rule::Offhand => Some(4),
                    _ => None,
                };
                if let Some(icon) = icon {
                    self.under
                        .blit(&s.icons[icon], sprites.texture, x, y, 16, 16, WHITE);
                    continue;
                }
            }
            if dragged {
                self.under.fill(x, y, x + 16, y + 16, DRAG);
            }
            if stack.is_empty() {
                continue;
            }
            let cooldown = v.cooldowns.get(i).map_or(0.0, |c| c.percent(d.partial));
            self.decorations(font, &stack, x, y, count, cooldown);
            self.item(stacks, stack, x, y, 0.0);
        }
        if let Some(h) = hovered {
            let (x, y, _) = v.slots[h];
            self.over.blit(
                &s.highlight_front,
                sprites.texture,
                left + x - 4,
                top + y - 4,
                24,
                24,
                WHITE,
            );
        }
        // `extractCarriedItem`: what a drag leaves on the cursor.
        if !carried.is_empty() {
            let shown = match drag {
                Some((slots, _, left)) if slots.len() > 1 => carried.copy_with_count(*left),
                _ => carried.clone(),
            };
            if !shown.is_empty() {
                let (x, y) = (d.mouse.0 - 8, d.mouse.1 - 8);
                self.decorations(font, &shown, x, y, None, 0.0);
                self.item(stacks, shown, x, y, CARRIED_Z);
            }
        }
        // `extractTooltip`: the hovered stack's name, when nothing is carried.
        if let Some(h) = hovered
            && carried.is_empty()
            && !v.stacks[h].is_empty()
        {
            let name = Arc::new(ruthenium_sim::item_name::styled_name(&v.stacks[h]));
            let line = memory.lines(&name, Wrap::Single, font, lang)[0].clone();
            self.tooltip(sprites, d, &line);
        }
        if let Some(cv) = &v.creative {
            self.creative_tooltips(sprites, font, lang, memory, d, cv, left, top);
        }
    }

    /// `GuiGraphicsExtractor.tooltip` of one line at the pointer
    /// (`DefaultTooltipPositioner`, `TooltipRenderUtil`).
    fn tooltip(
        &mut self,
        sprites: &HudSprites,
        d: &ContainerDraw<'_>,
        line: &Arc<super::TextLine>,
    ) {
        let s = &sprites.container;
        let (w, h) = (line.width, 8);
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
        self.styled(line, [0, 0], 1, x, y, WHITE, true);
    }
}
