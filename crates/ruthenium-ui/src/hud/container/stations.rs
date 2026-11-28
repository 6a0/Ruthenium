//! The beacon's, the brewing stand's, the enchanting table's, the grindstone's, the
//! mounts' and the lectern's screens, as vanilla draws what they add to a container
//! screen (`BeaconScreen`, `BrewingStandScreen`, `EnchantmentScreen`,
//! `GrindstoneScreen`, `AbstractMountInventoryScreen`) or draw instead
//! (`LecternScreen`'s `BookViewScreen`).
//!
//! Not drawn: the mount itself beside its slots and the enchanting table's book, which
//! are a model drawn in the GUI (no renderer for those yet); the enchanting table's
//! Standard Galactic words when the window has no `minecraft:alt` font.

use std::sync::{Arc, LazyLock};

use ruthenium_assets::Font;
use ruthenium_javamath::random::JavaRandom;
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::text::{Lang, Style};
use ruthenium_sim::item::{EquipmentSlot, Stack};
use ruthenium_sim::menu::{Kind, Rule, stations as menus};
use ruthenium_sim::session::{Button, ButtonKind, StationView};

use super::super::text::{HudMemory, TextLine, Wrap};
use super::super::{HudLayout, HudSprites, WHITE, text_width};
use super::{ContainerDraw, PICTURE};
use crate::draw::SpriteInfo;

/// `BrewingStandScreen.BUBBLELENGTHS`.
const BUBBLES: [i32; 7] = [29, 24, 20, 16, 11, 6, 0];
/// The beacon's labels' colour (`-2039584`).
const BEACON_LABEL: u32 = 0xFFE0_E0E0;
/// A book's text (`PAGE_TEXT_STYLE`: black, no shadow).
const PAGE_TEXT: u32 = 0xFF00_0000;

/// `EnchantmentNames.words`.
const WORDS: &[&str] = &[
    "the",
    "elder",
    "scrolls",
    "klaatu",
    "berata",
    "niktu",
    "xyzzy",
    "bless",
    "curse",
    "light",
    "darkness",
    "fire",
    "air",
    "earth",
    "water",
    "hot",
    "dry",
    "cold",
    "wet",
    "ignite",
    "snuff",
    "embiggen",
    "twist",
    "shorten",
    "stretch",
    "fiddle",
    "destroy",
    "imbue",
    "galvanize",
    "enchant",
    "free",
    "limited",
    "range",
    "of",
    "towards",
    "inside",
    "sphere",
    "cube",
    "self",
    "other",
    "ball",
    "mental",
    "physical",
    "grow",
    "shrink",
    "demon",
    "elemental",
    "spirit",
    "animal",
    "creature",
    "beast",
    "humanoid",
    "undead",
    "fresh",
    "stale",
    "phnglui",
    "mglwnafh",
    "cthulhu",
    "rlyeh",
    "wgahnagl",
    "fhtagn",
    "baguette",
];

/// The sprites these screens draw.
#[derive(Debug, Clone)]
pub(in crate::hud) struct StationSprites {
    /// A beacon's button: plain, highlighted, selected, disabled; then Done and Cancel.
    beacon: [SpriteInfo; 4],
    confirm: SpriteInfo,
    cancel: SpriteInfo,
    fuel_length: SpriteInfo,
    brew_progress: SpriteInfo,
    bubbles: SpriteInfo,
    grindstone_error: SpriteInfo,
    /// An enchanting table's offer: plain, disabled, highlighted; then its level, enabled
    /// and disabled.
    offer: [SpriteInfo; 3],
    levels: [[SpriteInfo; 3]; 2],
    slot: SpriteInfo,
    chest_slots: SpriteInfo,
    /// Empty slots: brewing fuel, potion, lapis, saddle, horse armour, llama armour,
    /// nautilus armour.
    icons: [SpriteInfo; 7],
    /// A book's page buttons: forward, highlighted; backward, highlighted.
    pages: [SpriteInfo; 4],
    /// `Button`: plain, disabled, highlighted.
    button: [SpriteInfo; 3],
}

impl StationSprites {
    pub(in crate::hud) fn with(s: &impl Fn(&str) -> SpriteInfo) -> StationSprites {
        let c = |n: &str| s(&format!("container/{n}"));
        StationSprites {
            beacon: [
                "button",
                "button_highlighted",
                "button_selected",
                "button_disabled",
            ]
            .map(|n| c(&format!("beacon/{n}"))),
            confirm: c("beacon/confirm"),
            cancel: c("beacon/cancel"),
            fuel_length: c("brewing_stand/fuel_length"),
            brew_progress: c("brewing_stand/brew_progress"),
            bubbles: c("brewing_stand/bubbles"),
            grindstone_error: c("grindstone/error"),
            offer: ["", "_disabled", "_highlighted"]
                .map(|n| c(&format!("enchanting_table/enchantment_slot{n}"))),
            levels: ["", "_disabled"]
                .map(|d| [1, 2, 3].map(|l| c(&format!("enchanting_table/level_{l}{d}")))),
            slot: c("slot"),
            chest_slots: c("horse/chest_slots"),
            icons: [
                "brewing_fuel",
                "potion",
                "lapis_lazuli",
                "saddle",
                "horse_armor",
                "llama_armor",
                "nautilus_armor_inventory",
            ]
            .map(|n| c(&format!("slot/{n}"))),
            pages: [
                "page_forward",
                "page_forward_highlighted",
                "page_backward",
                "page_backward_highlighted",
            ]
            .map(|n| s(&format!("widget/{n}"))),
            button: ["button", "button_disabled", "button_highlighted"]
                .map(|n| s(&format!("widget/{n}"))),
        }
    }

    /// `Slot.getNoItemIcon` of this module's slots.
    pub(in crate::hud) fn icon(&self, rule: Rule, kind: Kind) -> Option<&SpriteInfo> {
        Some(match rule {
            Rule::BrewingFuel => &self.icons[0],
            Rule::Potion => &self.icons[1],
            Rule::Lapis => &self.icons[2],
            Rule::MountEquipment(EquipmentSlot::Saddle) => &self.icons[3],
            Rule::MountEquipment(_) => match kind {
                Kind::Mount(m) if m.nautilus => &self.icons[6],
                Kind::Mount(m) if m.llama => &self.icons[5],
                _ => &self.icons[4],
            },
            _ => return None,
        })
    }
}

fn translated(key: &str) -> Arc<Tag> {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.into()));
    Arc::new(Tag::Compound(c))
}

/// `Component.translatable(key, args...)` in `colour` (a `ChatFormatting` name).
fn translated_with(key: &str, args: Vec<Tag>, colour: Option<&str>) -> Tag {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.into()));
    if !args.is_empty() {
        c.put("with", Tag::List(args));
    }
    if let Some(colour) = colour {
        c.put("color", Tag::String(colour.into()));
    }
    Tag::Compound(c)
}

static PRIMARY: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("block.minecraft.beacon.primary"));
static SECONDARY: LazyLock<Arc<Tag>> =
    LazyLock::new(|| translated("block.minecraft.beacon.secondary"));
static DONE: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("gui.done"));
static CANCEL: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("gui.cancel"));
static TAKE_BOOK: LazyLock<Arc<Tag>> = LazyLock::new(|| translated("lectern.take_book"));
/// Each effect's name by registry id, and with " II" (`BeaconUpgradePowerButton`).
static EFFECT_NAMES: LazyLock<Vec<[Arc<Tag>; 2]>> = LazyLock::new(|| {
    super::super::EFFECTS
        .iter()
        .map(|(name, ..)| {
            let key = format!("effect.minecraft.{name}");
            let mut two = Compound::new();
            two.put("translate", Tag::String(key.clone()));
            two.put("extra", Tag::List(vec![Tag::String(" II".into())]));
            [translated(&key), Arc::new(Tag::Compound(two))]
        })
        .collect()
});

/// `EnchantmentNames.getRandomName`: three or four of the words, as much of them as
/// `width` takes in the alt font (`headByWidth`).
pub fn random_name(random: &mut JavaRandom, font: &Font, width: i32) -> String {
    let n = random.next_int(2) + 3;
    let mut words = String::new();
    for i in 0..n {
        if i != 0 {
            words.push(' ');
        }
        words.push_str(WORDS[random.next_int(WORDS.len() as i32) as usize]);
    }
    let mut left = width as f32;
    let mut end = 0;
    for (i, c) in words.char_indices() {
        left -= font.glyph(c).advance(false);
        if left < 0.0 {
            break;
        }
        end = i + c.len_utf8();
    }
    words.truncate(end);
    words
}

impl HudLayout {
    /// A text line from `tag`'s first line.
    fn line(memory: &mut HudMemory, tag: &Arc<Tag>, font: &Font, lang: &dyn Lang) -> Arc<TextLine> {
        memory.lines(tag, Wrap::Single, font, lang)[0].clone()
    }

    /// What these screens draw over the window's picture, before the slots
    /// (`extractBackground` and the screens' buttons).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn station_background(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        memory: &mut HudMemory,
        lang: &dyn Lang,
        d: &ContainerDraw<'_>,
        stacks: &mut Vec<Stack>,
        (left, top): (i32, i32),
    ) {
        let s = &sprites.container.station;
        let t = sprites.texture;
        let v = d.view;
        match v.kind {
            Kind::BrewingStand => {
                let [ticks, fuel, total_ticks, total_fuel, ..] = v.data[..] else {
                    return;
                };
                let length = if total_fuel > 0 {
                    (-(-18 * fuel).div_euclid(total_fuel)).clamp(0, 18)
                } else {
                    0
                };
                if length > 0 {
                    let (x, y) = (left + 60, top + 44);
                    self.under.blit_part(
                        &s.fuel_length,
                        t,
                        [18, 4],
                        [0, 0],
                        x,
                        y,
                        length,
                        4,
                        WHITE,
                    );
                }
                if ticks > 0 && total_ticks > 0 {
                    let length = (28.0 * (1.0 - ticks as f32 / total_ticks as f32)) as i32;
                    if length > 0 {
                        let (x, y) = (left + 97, top + 16);
                        self.under.blit_part(
                            &s.brew_progress,
                            t,
                            [9, 28],
                            [0, 0],
                            x,
                            y,
                            9,
                            length,
                            WHITE,
                        );
                    }
                    let length = BUBBLES[(ticks / 2 % 7) as usize];
                    if length > 0 {
                        let (x, y) = (left + 63, top + 14 + 29 - length);
                        self.under.blit_part(
                            &s.bubbles,
                            t,
                            [12, 29],
                            [0, 29 - length],
                            x,
                            y,
                            12,
                            length,
                            WHITE,
                        );
                    }
                }
            }
            Kind::Grindstone => {
                let has = |i: usize| v.stacks.get(i).is_some_and(|s| !s.is_empty());
                if (has(0) || has(1)) && !has(2) {
                    self.under
                        .blit(&s.grindstone_error, t, left + 92, top + 31, 28, 21, WHITE);
                }
            }
            Kind::Mount(m) => {
                if m.columns > 0 && !m.nautilus {
                    let w = i32::from(m.columns) * 18;
                    self.under.blit_part(
                        &s.chest_slots,
                        t,
                        [90, 54],
                        [0, 0],
                        left + 79,
                        top + 17,
                        w,
                        54,
                        WHITE,
                    );
                }
                if m.saddle_shown {
                    self.under
                        .blit(&s.slot, t, left + 7, top + 17, 18, 18, WHITE);
                }
                if m.body_shown {
                    self.under
                        .blit(&s.slot, t, left + 7, top + 35, 18, 18, WHITE);
                }
            }
            Kind::Beacon => {
                if let Some(st) = &v.station {
                    let places = [20, 41, 63, 86, 108];
                    for (stack, x) in st.items.iter().zip(places) {
                        self.item(stacks, stack.clone(), left + x, top + 109, 0.0);
                    }
                    for b in st.buttons.iter().filter(|b| b.visible) {
                        let hovered = b.contains(f64::from(d.mouse.0), f64::from(d.mouse.1));
                        let ground = if !b.active {
                            &s.beacon[3]
                        } else if b.selected {
                            &s.beacon[2]
                        } else if hovered || b.focused {
                            &s.beacon[1]
                        } else {
                            &s.beacon[0]
                        };
                        self.under
                            .blit(ground, t, b.x, b.y, b.width, b.height, WHITE);
                        let icon = match b.kind {
                            ButtonKind::BeaconEffect { effect, .. } => usize::try_from(effect)
                                .ok()
                                .and_then(|e| sprites.effects.get(e)),
                            ButtonKind::BeaconConfirm => Some(&s.confirm),
                            _ => Some(&s.cancel),
                        };
                        if let Some(icon) = icon {
                            self.under.blit(icon, t, b.x + 2, b.y + 2, 18, 18, WHITE);
                        }
                    }
                }
            }
            Kind::Enchantment => {
                self.enchanting_offers(sprites, font, memory, lang, d, (left, top))
            }
            _ => {}
        }
    }

    /// `EnchantmentScreen.extractBackground`'s three offers.
    fn enchanting_offers(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        _memory: &mut HudMemory,
        _lang: &dyn Lang,
        d: &ContainerDraw<'_>,
        (left, top): (i32, i32),
    ) {
        let s = &sprites.container.station;
        let t = sprites.texture;
        let v = d.view;
        let Some(st) = &v.station else {
            return;
        };
        let seed = v.data.get(menus::ENCHANT_SEED).copied().unwrap_or(0);
        let mut random = JavaRandom::new(i64::from(seed));
        let lapis = v
            .stacks
            .get(1)
            .map_or(0, |s| if s.is_empty() { 0 } else { s.count });
        for i in 0..3 {
            let (x, y) = (left + 60, top + 14 + 19 * i as i32);
            let text_x = x + 20;
            let cost = v.data.get(menus::ENCHANT_COSTS + i).copied().unwrap_or(0);
            if cost == 0 {
                self.under.blit(&s.offer[1], t, x, y, 108, 19, WHITE);
                continue;
            }
            let cost_text = cost.to_string();
            let width = 86 - text_width(font, &cost_text);
            // (The words are drawn in the alt font: without it they take no room.)
            let words = d.alt_font.map(|alt| random_name(&mut random, alt, width));
            let mut colour: u32 = 0xFF68_5E4A;
            let cost_colour;
            if (lapis < i as i32 + 1 || st.experience_level < cost) && !st.infinite_materials {
                self.under.blit(&s.offer[1], t, x, y, 108, 19, WHITE);
                self.under
                    .blit(&s.levels[1][i], t, x + 1, y + 1, 16, 16, WHITE);
                colour = 0xFF00_0000 | (colour & 0x00FE_FEFE) >> 1;
                cost_colour = 0xFF40_7F10;
            } else {
                let (mx, my) = (d.mouse.0 - x, d.mouse.1 - y);
                if (0..108).contains(&mx) && (0..19).contains(&my) {
                    self.under.blit(&s.offer[2], t, x, y, 108, 19, WHITE);
                    colour = 0xFFFF_FF80;
                } else {
                    self.under.blit(&s.offer[0], t, x, y, 108, 19, WHITE);
                }
                self.under
                    .blit(&s.levels[0][i], t, x + 1, y + 1, 16, 16, WHITE);
                cost_colour = 0xFF80_FF20;
            }
            if let (Some(words), Some(alt)) = (words, d.alt_font) {
                self.alt_text(&words, alt, text_x, y + 2, colour);
            }
            let cost_x = text_x + 86 - text_width(font, &cost_text);
            self.text(
                format_args!("{cost_text}"),
                cost_x,
                y + 2 + 7,
                cost_colour,
                true,
            );
        }
    }

    /// A line in the alt font, without a shadow (`textWithWordWrap` of one line).
    fn alt_text(&mut self, words: &str, alt: &Font, x: i32, y: i32, colour: u32) {
        let chars: Vec<(char, Style)> = words.chars().map(|c| (c, Style::default())).collect();
        let width: f32 = chars
            .iter()
            .map(|(c, _)| alt.glyph(*c).advance(false))
            .sum();
        let line = Arc::new(TextLine {
            chars,
            width: width.ceil() as i32,
        });
        self.styled(&line, [0, 0], 1, x, y, colour, false);
        self.mark_alt();
    }

    /// The beacon's labels (`BeaconScreen.extractLabels`), which take the place of the
    /// title and the inventory's. Whether the screen has labels of its own.
    pub(super) fn station_labels(
        &mut self,
        font: &Font,
        memory: &mut HudMemory,
        lang: &dyn Lang,
        kind: Kind,
        (left, top): (i32, i32),
    ) -> bool {
        if kind != Kind::Beacon {
            return false;
        }
        for (label, x) in [(&*PRIMARY, 62), (&*SECONDARY, 169)] {
            let line = Self::line(memory, label, font, lang);
            self.styled(
                &line,
                [0, 0],
                1,
                left + x - line.width / 2,
                top + 10,
                BEACON_LABEL,
                true,
            );
        }
        true
    }

    /// A tooltip of these screens under the pointer: a beacon's button's
    /// (`WidgetTooltipHolder`, `MenuTooltipPositioner`), drawn before a slot's would
    /// be; an enchanting table's offer's (`EnchantmentScreen.extractRenderState`).
    /// Whether one was drawn.
    pub(super) fn station_tooltip(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        memory: &mut HudMemory,
        lang: &dyn Lang,
        d: &ContainerDraw<'_>,
        (left, top): (i32, i32),
    ) -> bool {
        let v = d.view;
        let Some(st) = &v.station else {
            return false;
        };
        let (mx, my) = d.mouse;
        match v.kind {
            Kind::Beacon => {
                let Some(b) = st
                    .buttons
                    .iter()
                    .find(|b| b.visible && b.contains(f64::from(mx), f64::from(my)))
                else {
                    return false;
                };
                let tag = match b.kind {
                    ButtonKind::BeaconEffect {
                        effect, upgrade, ..
                    } => {
                        match usize::try_from(effect)
                            .ok()
                            .and_then(|e| EFFECT_NAMES.get(e))
                        {
                            Some(names) => names[usize::from(upgrade)].clone(),
                            None => return false,
                        }
                    }
                    ButtonKind::BeaconConfirm => DONE.clone(),
                    _ => CANCEL.clone(),
                };
                let lines = memory.lines(&tag, Wrap::Split(170), font, lang);
                let (w, h) = tooltip_size(&lines);
                let at = menu_tooltip_place(d, b, (w, h));
                self.tooltip_lines(sprites, &lines, at);
                true
            }
            Kind::Enchantment => {
                for (i, clue) in st.clues.iter().enumerate() {
                    let Some(clue) = clue else {
                        continue;
                    };
                    let cost = v.data.get(menus::ENCHANT_COSTS + i).copied().unwrap_or(0);
                    let (x, y) = (mx - left, my - top);
                    // `isHovering(60, 14 + 19 * i, 108, 17)`: a unit more around.
                    let top_edge = 13 + 19 * i as i32;
                    let over = (59..169).contains(&x) && (top_edge..top_edge + 19).contains(&y);
                    if !over || cost <= 0 || clue.level < 0 {
                        continue;
                    }
                    let tags = offer_tooltip(clue, i as i32 + 1, cost, st, v.stacks.get(1));
                    let lines: Vec<Arc<TextLine>> = tags
                        .iter()
                        .map(|t| memory.lines(t, Wrap::Single, font, lang)[0].clone())
                        .collect();
                    let (w, h) = tooltip_size(&lines);
                    let at = default_tooltip_place(d, (w, h));
                    self.tooltip_lines(sprites, &lines, at);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// `GuiGraphicsExtractor.tooltip` of these lines at `(x, y)` (no gap after the
    /// first line).
    fn tooltip_lines(&mut self, sprites: &HudSprites, lines: &[Arc<TextLine>], (x, y): (i32, i32)) {
        let s = &sprites.container;
        let (w, h) = tooltip_size(lines);
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
        for (i, line) in lines.iter().enumerate() {
            self.styled(line, [0, 0], 1, x, y + 10 * i as i32, WHITE, true);
        }
    }

    /// The lectern's book screen (`BookViewScreen.extractBackground`,
    /// `extractRenderState`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lectern(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        memory: &mut HudMemory,
        d: &ContainerDraw<'_>,
    ) {
        let s = &sprites.container.station;
        let t = sprites.texture;
        let (left, top) = ((d.width - 192) / 2, 2);
        if let Some(book) = d.background {
            self.under.blit_part(
                &PICTURE,
                book,
                [256, 256],
                [0, 0],
                left,
                top,
                192,
                192,
                WHITE,
            );
        }
        let Some(st) = &d.view.station else {
            return;
        };
        let (mx, my) = (f64::from(d.mouse.0), f64::from(d.mouse.1));
        for b in st.buttons.iter().filter(|b| b.visible) {
            let hovered = b.contains(mx, my);
            match b.kind {
                ButtonKind::PageForward | ButtonKind::PageBack => {
                    let back = usize::from(b.kind == ButtonKind::PageBack) * 2;
                    let sprite = &s.pages[back + usize::from(hovered)];
                    self.under.blit(sprite, t, b.x, b.y, 23, 13, WHITE);
                }
                ButtonKind::Done | ButtonKind::TakeBook => {
                    let sprite = match (b.active, hovered || b.focused) {
                        (false, _) => &s.button[1],
                        (true, false) => &s.button[0],
                        (true, true) => &s.button[2],
                    };
                    self.under
                        .blit(sprite, t, b.x, b.y, b.width, b.height, WHITE);
                    let label = if b.kind == ButtonKind::Done {
                        &*DONE
                    } else {
                        &*TAKE_BOOK
                    };
                    let line = Self::line(memory, label, font, lang);
                    // `acceptScrollingWithDefaultCenter` of a label that fits.
                    let (l, r) = (b.x + 2, b.x + b.width - 2);
                    let x = ((l + r) / 2).clamp(l + line.width / 2, r - line.width / 2);
                    let y = (b.y + b.y + b.height - 9) / 2 + 1;
                    let colour = if b.active { WHITE } else { 0xFFA0_A0A0 };
                    self.styled(&line, [0, 0], 1, x - line.width / 2, y, colour, true);
                }
                _ => {}
            }
        }
        // `getPageNumberMessage`, right-aligned; then the page's lines.
        let count = st.pages.len() as i32;
        let indicator = Arc::new(translated_with(
            "book.pageIndicator",
            vec![
                Tag::String((st.page + 1).to_string()),
                Tag::String(count.max(1).to_string()),
            ],
            None,
        ));
        let line = memory.lines(&indicator, Wrap::Single, font, lang)[0].clone();
        self.styled(
            &line,
            [0, 0],
            1,
            left + 148 - line.width,
            top + 16,
            PAGE_TEXT,
            false,
        );
        let Some(page) = usize::try_from(st.page).ok().and_then(|p| st.pages.get(p)) else {
            return;
        };
        let lines = memory.lines(page, Wrap::Split(114), font, lang);
        for (i, line) in lines.iter().take(128 / 9).enumerate() {
            self.styled(
                line,
                [0, 0],
                1,
                left + 36,
                top + 30 + i as i32 * 9,
                PAGE_TEXT,
                false,
            );
        }
    }
}

/// A tooltip's size: its widest line, and ten units a line less two.
fn tooltip_size(lines: &[Arc<TextLine>]) -> (i32, i32) {
    let w = lines.iter().map(|l| l.width).max().unwrap_or(0);
    (w, lines.len() as i32 * 10 - 2)
}

/// `DefaultTooltipPositioner`.
fn default_tooltip_place(d: &ContainerDraw<'_>, (w, h): (i32, i32)) -> (i32, i32) {
    let (mut x, mut y) = (d.mouse.0 + 12, d.mouse.1 - 12);
    if x + w > d.width {
        x = (x - 24 - w).max(4);
    }
    if y + h + 3 > d.height {
        y = d.height - (h + 3);
    }
    (x, y)
}

/// `MenuTooltipPositioner` beside a widget.
fn menu_tooltip_place(d: &ContainerDraw<'_>, b: &Button, (w, h): (i32, i32)) -> (i32, i32) {
    let (mx, my) = d.mouse;
    let offset = |mouse: i32, widget: i32, height: i32| {
        let distance = (mouse - widget).abs().min(height);
        let f = distance as f32 / height as f32;
        (height as f32 - 3.0 + f * (5.0 - (height as f32 - 3.0))).round() as i32
    };
    let mut x = mx + 12;
    if x + w > d.width - 5 {
        x = (mx - 12 - w).max(9);
    }
    let mut y = my + 3;
    let padded = h + 3 + 3;
    let lowest = b.y + b.height + 3 + offset(0, 0, b.height);
    if lowest + padded <= d.height - 5 {
        y += offset(y, b.y, b.height);
    } else {
        y -= padded + offset(y, b.y + b.height, b.height);
    }
    (x, y)
}

/// An enchanting table's offer's tooltip: what it gives (`container.enchant.clue`
/// with `Enchantment.getFullname`), then what it costs unless materials are infinite.
fn offer_tooltip(
    clue: &ruthenium_sim::session::EnchantClue,
    lapis_cost: i32,
    level_cost: i32,
    st: &StationView,
    lapis: Option<&Stack>,
) -> Vec<Arc<Tag>> {
    // `getFullname`: the name in red for a curse, else grey, where it has no colour of
    // its own, and the level unless there is only one.
    let mut name = match clue.name.as_deref() {
        Some(Tag::Compound(c)) => c.clone(),
        Some(Tag::String(s)) => {
            let mut c = Compound::new();
            c.put("text", Tag::String(s.clone()));
            c
        }
        _ => {
            let mut c = Compound::new();
            c.put("text", Tag::String(String::new()));
            c
        }
    };
    if name.get("color").is_none() {
        name.put(
            "color",
            Tag::String(if clue.curse { "red" } else { "gray" }.into()),
        );
    }
    if clue.level != 1 || clue.max_level != 1 {
        let mut extra = name.list("extra").to_vec();
        extra.push(Tag::String(" ".into()));
        extra.push(translated_with(
            &format!("enchantment.level.{}", clue.level),
            Vec::new(),
            None,
        ));
        name.put("extra", Tag::List(extra));
    }
    let mut lines = vec![Arc::new(translated_with(
        "container.enchant.clue",
        vec![Tag::Compound(name)],
        Some("white"),
    ))];
    if st.infinite_materials {
        return lines;
    }
    lines.push(Arc::new(Tag::String(String::new())));
    if st.experience_level < level_cost {
        lines.push(Arc::new(translated_with(
            "container.enchant.level.requirement",
            vec![Tag::String(level_cost.to_string())],
            Some("red"),
        )));
        return lines;
    }
    let lapis = lapis.map_or(0, |s| if s.is_empty() { 0 } else { s.count });
    let (lapis_key, level_key, args) = if lapis_cost == 1 {
        (
            "container.enchant.lapis.one",
            "container.enchant.level.one",
            Vec::new(),
        )
    } else {
        (
            "container.enchant.lapis.many",
            "container.enchant.level.many",
            vec![Tag::String(lapis_cost.to_string())],
        )
    };
    let colour = if lapis >= lapis_cost { "gray" } else { "red" };
    lines.push(Arc::new(translated_with(
        lapis_key,
        args.clone(),
        Some(colour),
    )));
    lines.push(Arc::new(translated_with(level_key, args, Some("gray"))));
    lines
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ruthenium_assets::font::Glyph;
    use ruthenium_assets::meta::GuiScaling;
    use ruthenium_sim::menu::mount::{Mount, MountState};
    use ruthenium_sim::menu::{Kind, Menu};
    use ruthenium_sim::session::{Button, ButtonKind, ContainerView, StationView};

    use super::*;
    use crate::draw::DrawList;

    /// Sprites told apart by their coordinates, as the HUD's tests do.
    struct Names(RefCell<Vec<String>>);

    impl Names {
        fn sprites(&self) -> HudSprites {
            HudSprites::with(7, |id| {
                let mut names = self.0.borrow_mut();
                names.push(id.path().to_owned());
                let n = (names.len() - 1) as f32;
                SpriteInfo {
                    uv: [n, 0.0, n + 1.0, 1.0],
                    width: 16,
                    height: 16,
                    scaling: GuiScaling::Stretch,
                }
            })
        }

        /// Each quad of a sprite named `name`: left, top, width, height.
        fn of(&self, list: &DrawList, name: &str) -> Vec<[i32; 4]> {
            let names = self.0.borrow();
            list.quads()
                .iter()
                .filter(|q| q.texture == 7 && names[q.uv[0].floor() as usize] == name)
                .map(|q| {
                    let [x0, y0, x1, y1] = q.rect.map(|v| v as i32);
                    [x0, y0, x1 - x0, y1 - y0]
                })
                .collect()
        }
    }

    /// Every character six wide, a space four.
    fn font() -> Font {
        let glyph = |advance| Glyph {
            advance,
            ..Glyph::MISSING
        };
        Font::of_glyphs((' '..='~').map(|c| (c, glyph(if c == ' ' { 4.0 } else { 6.0 }))))
    }

    fn view(kind: Kind, data: Vec<i32>, station: Option<StationView>) -> ContainerView {
        let menu = Menu::new(kind, 1);
        let (image_width, image_height) = ruthenium_sim::menu::screen::image_size(kind);
        ContainerView {
            serial: 1,
            kind,
            title: Some(Arc::new(Tag::String("Title".into()))),
            image_width,
            image_height,
            slots: menu.slots.iter().map(|s| (s.x, s.y, s.rule)).collect(),
            stacks: vec![Stack::empty(); menu.slots.len()],
            carried: Stack::empty(),
            screen: ruthenium_sim::menu::screen::View {
                left: 0,
                top: 0,
                hovered: None,
                quick_craft: None,
            },
            data,
            cooldowns: vec![Default::default(); menu.slots.len()],
            creative: None,
            station,
        }
    }

    /// Lays out `v` on a 400 by 300 GUI with the pointer at `mouse`; the window's
    /// corner.
    fn draw(names: &Names, v: &ContainerView, mouse: (i32, i32)) -> (HudLayout, (i32, i32)) {
        let sprites = names.sprites();
        let (font, alt) = (font(), font());
        let mut layout = HudLayout::default();
        let d = ContainerDraw {
            view: v,
            width: 400,
            height: 300,
            mouse,
            background: Some(3),
            partial: 0.0,
            alt_font: Some(&alt),
        };
        layout.container(
            &sprites,
            &font,
            &(),
            &mut HudMemory::default(),
            &d,
            &mut Vec::new(),
        );
        (
            layout,
            ((400 - v.image_width) / 2, (300 - v.image_height) / 2),
        )
    }

    /// The texts laid out: what, where, colour and whether in the alt font.
    fn texts(l: &HudLayout) -> Vec<(String, i32, i32, u32, bool)> {
        l.texts[..l.texts_used]
            .iter()
            .map(|t| {
                let s = match &t.line {
                    Some(line) => line.chars.iter().map(|(c, _)| *c).collect(),
                    None => t.text.clone(),
                };
                (s, t.x as i32, t.y as i32, t.color, t.alt)
            })
            .collect()
    }

    #[test]
    fn a_brewing_stand_shows_its_fuel_and_its_progress() {
        let names = Names(RefCell::new(Vec::new()));
        // 100 of 400 ticks left, 10 of 20 fuel.
        let v = view(Kind::BrewingStand, vec![100, 10, 400, 20], None);
        let (l, (x, y)) = draw(&names, &v, (0, 0));
        let under = &l.under;
        assert_eq!(
            names.of(under, "container/brewing_stand/fuel_length"),
            [[x + 60, y + 44, 9, 4]]
        );
        assert_eq!(
            names.of(under, "container/brewing_stand/brew_progress"),
            [[x + 97, y + 16, 9, 21]]
        );
        // Ticks 100: the second bubble picture, 24 high.
        assert_eq!(
            names.of(under, "container/brewing_stand/bubbles"),
            [[x + 63, y + 19, 12, 24]]
        );
        // Empty: the potions' and the fuel's icons; the title in the middle.
        assert_eq!(names.of(under, "container/slot/potion").len(), 3);
        assert_eq!(
            names.of(under, "container/slot/brewing_fuel"),
            [[x + 17, y + 17, 16, 16]]
        );
        let t = texts(&l);
        assert_eq!((t[0].0.as_str(), t[0].1), ("Title", x + (176 - 30) / 2));
        // Not brewing: no arrow and no bubbles.
        let v = view(Kind::BrewingStand, vec![0, 0, 0, 0], None);
        let (l, _) = draw(&names, &v, (0, 0));
        assert!(
            names
                .of(&l.under, "container/brewing_stand/bubbles")
                .is_empty()
        );
        assert!(
            names
                .of(&l.under, "container/brewing_stand/fuel_length")
                .is_empty()
        );
    }

    #[test]
    fn a_mounts_window_shows_its_chest_and_what_it_may_wear() {
        let names = Names(RefCell::new(Vec::new()));
        let state = MountState {
            alive: true,
            baby: false,
            tamed: true,
        };
        let llama = Mount::new("minecraft:llama", 79, 3, state, &Default::default()).unwrap();
        let mut v = view(Kind::Mount(llama), Vec::new(), None);
        // The saddle slot isn't there for a llama.
        v.slots[0].0 = ruthenium_sim::menu::HIDDEN;
        let (l, (x, y)) = draw(&names, &v, (0, 0));
        assert_eq!(
            names.of(&l.under, "container/horse/chest_slots"),
            [[x + 79, y + 17, 54, 54]]
        );
        assert_eq!(
            names.of(&l.under, "container/slot"),
            [[x + 7, y + 35, 18, 18]]
        );
        assert_eq!(
            names.of(&l.under, "container/slot/llama_armor"),
            [[x + 8, y + 36, 16, 16]]
        );
        assert!(names.of(&l.under, "container/slot/saddle").is_empty());
        let nautilus = Mount::new("minecraft:nautilus", 89, 0, state, &Default::default()).unwrap();
        let v = view(Kind::Mount(nautilus), Vec::new(), None);
        let (l, (x, y)) = draw(&names, &v, (0, 0));
        assert!(names.of(&l.under, "container/horse/chest_slots").is_empty());
        assert_eq!(names.of(&l.under, "container/slot").len(), 2);
        assert_eq!(
            names.of(&l.under, "container/slot/saddle"),
            [[x + 8, y + 18, 16, 16]]
        );
        assert_eq!(
            names
                .of(&l.under, "container/slot/nautilus_armor_inventory")
                .len(),
            1
        );
    }

    fn station() -> StationView {
        StationView {
            experience_level: 10,
            ..StationView::default()
        }
    }

    #[test]
    fn an_enchanting_tables_offers_are_coloured_by_what_they_cost() {
        let names = Names(RefCell::new(Vec::new()));
        let mut data = vec![0, 5, 30, 1234, -1, -1, -1, -1, -1, -1];
        let mut v = view(Kind::Enchantment, data.clone(), Some(station()));
        v.stacks[1] = Stack::empty();
        let (l, (x, y)) = draw(&names, &v, (0, 0));
        let slot = |n: &str| names.of(&l.under, &format!("container/enchanting_table/{n}"));
        // No lapis: every offer is off; the first costs nothing and shows nothing.
        assert_eq!(slot("enchantment_slot_disabled").len(), 3);
        assert_eq!(slot("level_2_disabled"), [[x + 61, y + 34, 16, 16]]);
        let t = texts(&l);
        let cost = t.iter().find(|t| t.0 == "5").unwrap();
        assert_eq!(
            (cost.1, cost.2, cost.3),
            (x + 80 + 86 - 6, y + 14 + 19 + 9, 0xFF40_7F10)
        );
        // The words, in the alt font, no wider than the room the cost leaves.
        let words = t.iter().find(|t| t.4).unwrap();
        assert_eq!((words.1, words.2), (x + 80, y + 35));
        assert_eq!(words.3, 0xFF00_0000 | (0x68_5E4A & 0xFE_FEFE) >> 1);
        let width: usize = words.0.chars().map(|c| if c == ' ' { 4 } else { 6 }).sum();
        assert!(width <= 80 && width > 80 - 6 - 4, "{}", words.0);
        // The same seed, the same words.
        let again = texts(&draw(&names, &v, (0, 0)).0);
        assert_eq!(again.iter().find(|t| t.4).unwrap().0, words.0);
        // Lapis enough for the second, levels enough: it is on, and lit under the
        // pointer, its words in yellow.
        data[0] = 0;
        let mut v = view(Kind::Enchantment, data, Some(station()));
        v.stacks[1] = ruthenium_sim::item::Items::default().new_stack(1014, 2);
        let (l, _) = draw(&names, &v, (x + 70, y + 40));
        let slot = |n: &str| names.of(&l.under, &format!("container/enchanting_table/{n}"));
        assert_eq!(
            slot("enchantment_slot_highlighted"),
            [[x + 60, y + 33, 108, 19]]
        );
        assert_eq!(slot("level_2"), [[x + 61, y + 34, 16, 16]]);
        let t = texts(&l);
        assert_eq!(t.iter().find(|t| t.0 == "5").unwrap().3, 0xFF80_FF20);
        assert_eq!(t.iter().find(|t| t.4).unwrap().3, 0xFFFF_FF80);
    }

    #[test]
    fn a_beacons_buttons_show_what_they_are() {
        let names = Names(RefCell::new(Vec::new()));
        let button = |kind, x, y, active, selected| Button {
            kind,
            x,
            y,
            width: 22,
            height: 22,
            visible: true,
            active,
            selected,
            focused: false,
        };
        let (bx, by) = ((400 - 230) / 2, (300 - 219) / 2);
        let speed = ButtonKind::BeaconEffect {
            effect: 0,
            primary: true,
            tier: 0,
            upgrade: false,
        };
        let haste = ButtonKind::BeaconEffect {
            effect: 2,
            primary: true,
            tier: 0,
            upgrade: false,
        };
        let st = StationView {
            buttons: vec![
                button(speed, bx + 53, by + 22, true, true),
                button(haste, bx + 77, by + 22, true, false),
                button(ButtonKind::BeaconConfirm, bx + 164, by + 107, false, false),
            ],
            ..station()
        };
        let v = view(Kind::Beacon, vec![1, 1, 0], Some(st));
        // Over haste: lit, with its name beside it.
        let (l, (x, y)) = draw(&names, &v, (bx + 80, by + 30));
        let b = |n: &str| names.of(&l.under, &format!("container/beacon/{n}"));
        assert_eq!(b("button_selected"), [[x + 53, y + 22, 22, 22]]);
        assert_eq!(b("button_highlighted"), [[x + 77, y + 22, 22, 22]]);
        assert_eq!(b("button_disabled"), [[x + 164, y + 107, 22, 22]]);
        assert_eq!(
            names.of(&l.under, "mob_effect/haste"),
            [[x + 79, y + 24, 18, 18]]
        );
        assert_eq!(b("confirm"), [[x + 166, y + 109, 18, 18]]);
        let t = texts(&l);
        // The two labels instead of the title, and the tooltip last.
        assert!(!t.iter().any(|t| t.0 == "Title"));
        let primary = &t[0];
        assert_eq!(
            (primary.1, primary.2, primary.3),
            (x + 62 - primary.0.len() as i32 * 3, y + 10, 0xFFE0_E0E0)
        );
        assert_eq!(t.last().unwrap().0, "effect.minecraft.haste");
        assert_eq!(names.of(&l.over, "tooltip/background").len(), 1);
    }

    #[test]
    fn a_lecterns_book_shows_its_page() {
        let names = Names(RefCell::new(Vec::new()));
        let pages: Arc<[Arc<Tag>]> = vec![
            Arc::new(Tag::String("first".into())),
            Arc::new(Tag::String(
                "a second page that is longer than one line of it".into(),
            )),
        ]
        .into();
        let (bl, bt) = ((400 - 192) / 2, 2);
        let st = StationView {
            pages,
            page: 1,
            buttons: vec![
                Button {
                    kind: ButtonKind::PageBack,
                    x: bl + 43,
                    y: bt + 157,
                    width: 23,
                    height: 13,
                    visible: true,
                    active: true,
                    selected: false,
                    focused: false,
                },
                Button {
                    kind: ButtonKind::Done,
                    x: 200 - 98 - 2,
                    y: 196,
                    width: 98,
                    height: 20,
                    visible: true,
                    active: true,
                    selected: false,
                    focused: false,
                },
            ],
            ..station()
        };
        let v = view(Kind::Lectern, vec![1], Some(st));
        let (l, _) = draw(&names, &v, (bl + 50, bt + 160));
        assert_eq!(l.under.quads().iter().filter(|q| q.texture == 3).count(), 1);
        assert_eq!(
            names.of(&l.under, "widget/page_backward_highlighted"),
            [[bl + 43, bt + 157, 23, 13]]
        );
        assert_eq!(names.of(&l.under, "widget/button"), [[100, 196, 98, 20]]);
        let t = texts(&l);
        // Done in the middle of its button; the indicator to the right; the page's
        // lines 114 wide at most, in black.
        let done = t.iter().find(|t| t.0 == "gui.done").unwrap();
        assert_eq!((done.1, done.2), (100 + 49 - 24, 196 + 6));
        let indicator = t
            .iter()
            .find(|t| t.0.starts_with("book.pageIndicator"))
            .unwrap();
        assert_eq!(indicator.1 + indicator.0.len() as i32 * 6, bl + 148);
        let lines: Vec<_> = t.iter().filter(|t| t.1 == bl + 36).collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            (lines[0].2, lines[1].2, lines[0].3),
            (bt + 30, bt + 39, 0xFF00_0000)
        );
        assert!(lines.iter().all(|l| l.0.len() * 6 <= 114));
        // No slots: nothing of a window.
        assert!(
            names
                .of(&l.under, "container/slot_highlight_back")
                .is_empty()
        );
    }
}
