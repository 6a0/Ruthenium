//! The HUD (`client/gui/Hud.java`, `contextualbar/ExperienceBar.java`, the item
//! decorations of `GuiGraphicsExtractor`), laid out as vanilla lays it out, in GUI units:
//! the crosshair and the attack indicator, the hotbar with what is in it, hearts, armour,
//! food, air, the experience bar and its level, the effects' icons, and what it says in
//! words (the `text` module).
//!
//! What comes out is lists for the renderer, in the order they are drawn: the crosshair
//! (which inverts what is behind it), the quads under the hotbar's items, the items, and
//! the quads over them, with the texts and the faces each between two of those quads.

mod container;
mod text;

use std::sync::Arc;
use std::time::Instant;

use ruthenium_assets::{Font, Id};
use ruthenium_javamath::mth;
use ruthenium_javamath::random::JavaRandom;
use ruthenium_protocol::text::Lang;
use ruthenium_sim::hud::{HudEffect, HudRecord, OFF_HAND};

pub use self::container::{
    BACKGROUNDS, ContainerDraw, background_name, background_path, view_background,
};
use self::text::Words;
pub use self::text::{HudFace, HudMemory, HudText, TextLine};
use crate::draw::{DrawList, SpriteInfo, Sprites};

const WHITE: u32 = 0xFFFF_FFFF;
const BLACK: u32 = 0xFF00_0000;
/// The experience level's own green.
const LEVEL_COLOUR: u32 = 0xFF80_FF20;
/// What lies over an item that cannot be used yet.
const COOLDOWN_COLOUR: u32 = 0x7FFF_FFFF;

/// Every effect by its registry id: its name, whether it is good for who has it
/// (`MobEffectCategory.BENEFICIAL`), and its colour, by which effects that end together
/// are ordered.
const EFFECTS: [(&str, bool, i32); 40] = [
    ("speed", true, 0x33EBFF),
    ("slowness", false, 0x8BAFE0),
    ("haste", true, 0xD9C043),
    ("mining_fatigue", false, 0x4A4217),
    ("strength", true, 0xFFC700),
    ("instant_health", true, 0xF82423),
    ("instant_damage", false, 0xA9656A),
    ("jump_boost", true, 0xFDFF84),
    ("nausea", false, 0x551D4A),
    ("regeneration", true, 0xCD5CAB),
    ("resistance", true, 0x9146F0),
    ("fire_resistance", true, 0xFF9900),
    ("water_breathing", true, 0x98DAC0),
    ("invisibility", true, 0xF6F6F6),
    ("blindness", false, 0x1F1F23),
    ("night_vision", true, 0xC2FF66),
    ("hunger", false, 0x587653),
    ("weakness", false, 0x484D48),
    ("poison", false, 0x87A363),
    ("wither", false, 0x736156),
    ("health_boost", true, 0xF87D23),
    ("absorption", true, 0x2552A5),
    ("saturation", true, 0xF82423),
    ("glowing", false, 0x94A061),
    ("levitation", false, 0xCEFFFF),
    ("luck", true, 0x59C106),
    ("unluck", false, 0xC0A44D),
    ("slow_falling", true, 0xF3CFB9),
    ("conduit_power", true, 0x1DC2D1),
    ("dolphins_grace", true, 0x88A3BE),
    ("bad_omen", false, 0x0B6138),
    ("hero_of_the_village", true, 0x44FF44),
    ("darkness", false, 0x292721),
    ("trial_omen", false, 0x16A6A6),
    ("raid_omen", false, 0xDE4058),
    ("wind_charged", false, 0xBDC9FF),
    ("weaving", false, 0x78695A),
    ("oozing", false, 0x99FFA3),
    ("infested", false, 0x8C9B8C),
    ("breath_of_the_nautilus", true, 0x00FFEE),
];

/// `BossBarColor`, by its ordinal.
const BOSS_COLOURS: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];

const REGENERATION: i32 = 9;
const HUNGER: i32 = 16;
const POISON: i32 = 18;
const WITHER: i32 = 19;

/// Where the attack indicator shows (the option of the same name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttackIndicator {
    Off,
    #[default]
    Crosshair,
    Hotbar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudOptions {
    pub attack_indicator: AttackIndicator,
    /// Whether the crosshair is drawn, and the attack indicator under it.
    pub crosshair: bool,
}

impl Default for HudOptions {
    fn default() -> HudOptions {
        HudOptions {
            attack_indicator: AttackIndicator::default(),
            crosshair: true,
        }
    }
}

/// A heart's kind (`Hud.HeartType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Heart {
    Container,
    Normal,
    Poisoned,
    Withered,
    Absorbing,
    Frozen,
}

impl Heart {
    const ALL: [Heart; 6] = [
        Heart::Container,
        Heart::Normal,
        Heart::Poisoned,
        Heart::Withered,
        Heart::Absorbing,
        Heart::Frozen,
    ];

    /// The sprite's path: a container has no halves.
    fn sprite(self, hardcore: bool, half: bool, blinking: bool) -> String {
        let blinking = if blinking { "_blinking" } else { "" };
        let kind = match self {
            Heart::Container => {
                let hardcore = if hardcore { "_hardcore" } else { "" };
                return format!("hud/heart/container{hardcore}{blinking}");
            }
            Heart::Normal => "",
            Heart::Poisoned => "poisoned_",
            Heart::Withered => "withered_",
            Heart::Absorbing => "absorbing_",
            Heart::Frozen => "frozen_",
        };
        let hardcore = if hardcore { "hardcore_" } else { "" };
        let half = if half { "half" } else { "full" };
        format!("hud/heart/{kind}{hardcore}{half}{blinking}")
    }
}

/// The HUD's sprites, looked up once.
#[derive(Debug, Clone)]
pub struct HudSprites {
    texture: u32,
    crosshair: SpriteInfo,
    crosshair_full: SpriteInfo,
    crosshair_background: SpriteInfo,
    crosshair_progress: SpriteInfo,
    effect_background: SpriteInfo,
    effect_background_ambient: SpriteInfo,
    hotbar: SpriteInfo,
    hotbar_selection: SpriteInfo,
    hotbar_offhand_left: SpriteInfo,
    hotbar_offhand_right: SpriteInfo,
    hotbar_attack_background: SpriteInfo,
    hotbar_attack_progress: SpriteInfo,
    /// Empty, half, full.
    armor: [SpriteInfo; 3],
    /// Empty, half, full; then the same when hungry.
    food: [[SpriteInfo; 3]; 2],
    /// Full, bursting, empty.
    air: [SpriteInfo; 3],
    /// By kind, then by hardcore, half and blinking as bits 4, 2 and 1.
    hearts: [[SpriteInfo; 8]; 6],
    experience_background: SpriteInfo,
    experience_progress: SpriteInfo,
    /// By the effect's registry id.
    effects: Vec<SpriteInfo>,
    /// By colour, then by notches: the bars' ground and what fills them.
    boss_bars: [[SpriteInfo; 2]; 7],
    boss_notches: [[SpriteInfo; 2]; 4],
    /// Unknown, then one to five bars.
    ping: [SpriteInfo; 6],
    missing: SpriteInfo,
    container: self::container::ContainerSprites,
}

impl HudSprites {
    pub fn new(sprites: &Sprites) -> HudSprites {
        HudSprites::with(sprites.texture(), |id| *sprites.get(id))
    }

    /// From any look-up of sprites by id, of the texture `texture`.
    pub fn with(texture: u32, get: impl Fn(&Id) -> SpriteInfo) -> HudSprites {
        let s = |path: &str| get(&Id::minecraft(path).expect("a valid path"));
        HudSprites {
            texture,
            crosshair: s("hud/crosshair"),
            crosshair_full: s("hud/crosshair_attack_indicator_full"),
            crosshair_background: s("hud/crosshair_attack_indicator_background"),
            crosshair_progress: s("hud/crosshair_attack_indicator_progress"),
            effect_background: s("hud/effect_background"),
            effect_background_ambient: s("hud/effect_background_ambient"),
            hotbar: s("hud/hotbar"),
            hotbar_selection: s("hud/hotbar_selection"),
            hotbar_offhand_left: s("hud/hotbar_offhand_left"),
            hotbar_offhand_right: s("hud/hotbar_offhand_right"),
            hotbar_attack_background: s("hud/hotbar_attack_indicator_background"),
            hotbar_attack_progress: s("hud/hotbar_attack_indicator_progress"),
            armor: ["empty", "half", "full"].map(|n| s(&format!("hud/armor_{n}"))),
            food: ["", "_hunger"].map(|hunger| {
                ["empty", "half", "full"].map(|n| s(&format!("hud/food_{n}{hunger}")))
            }),
            air: ["hud/air", "hud/air_bursting", "hud/air_empty"].map(s),
            hearts: Heart::ALL.map(|kind| {
                std::array::from_fn(|i| s(&kind.sprite(i & 4 != 0, i & 2 != 0, i & 1 != 0)))
            }),
            experience_background: s("hud/experience_bar_background"),
            experience_progress: s("hud/experience_bar_progress"),
            effects: EFFECTS
                .iter()
                .map(|(name, ..)| s(&format!("mob_effect/{name}")))
                .collect(),
            boss_bars: BOSS_COLOURS
                .map(|c| ["background", "progress"].map(|part| s(&format!("boss_bar/{c}_{part}")))),
            boss_notches: [6, 10, 12, 20].map(|n| {
                ["background", "progress"].map(|part| s(&format!("boss_bar/notched_{n}_{part}")))
            }),
            ping: ["unknown", "1", "2", "3", "4", "5"].map(|n| s(&format!("icon/ping_{n}"))),
            missing: s("missingno"),
            container: self::container::ContainerSprites::with(&s),
        }
    }

    fn heart(&self, kind: Heart, hardcore: bool, half: bool, blinking: bool) -> &SpriteInfo {
        let i = usize::from(hardcore) << 2 | usize::from(half) << 1 | usize::from(blinking);
        &self.hearts[kind as usize][i]
    }
}

/// What the hearts remember between frames (`Hud`'s `lastHealth`, `displayHealth`,
/// `lastHealthTime` and `healthBlinkTime`): they blink when the health changes, and show
/// what was lost for a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HudHealth {
    last_health: i32,
    display_health: i32,
    /// In milliseconds, on the clock the frames are laid out with.
    last_health_time: i64,
    /// The HUD's tick the blinking ends at.
    blink_time: i64,
}

/// An item of the hotbar: which of the record's slots, and where its 16 by 16 units are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudItem {
    pub slot: usize,
    pub x: i32,
    pub y: i32,
    /// A stack that just grew is squeezed sideways and stretched upwards about the
    /// middle of its lower part, `(x + 8, y + 12)`: the two factors.
    pub squeeze: Option<[f32; 2]>,
    /// How much nearer than the others it is drawn, in GUI units (a stack on the
    /// cursor over the slots').
    pub z: f32,
}

/// What a frame's HUD is laid out from.
#[derive(Debug, Clone, Copy)]
pub struct HudView<'a> {
    pub hud: &'a HudRecord,
    /// How far the record's tick has come.
    pub partial: f32,
    /// The GUI's size in its own units.
    pub width: i32,
    pub height: i32,
    /// A clock in milliseconds (`Util.getMillis`), and the time itself.
    pub millis: i64,
    pub now: Instant,
    pub options: HudOptions,
    /// The chat screen, while it is open over the HUD.
    pub chat: Option<&'a ChatInput>,
}

/// The chat screen as the HUD draws it (`ChatScreen`): what is typed, the caret and
/// the selection's other end (byte offsets), whether the caret blinks on, and how far
/// the chat is scrolled up, in lines, as of the newest message then.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatInput {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
    pub scroll: i32,
    pub newest: Option<Arc<ruthenium_sim::hud_text::ChatMessage>>,
    /// A message came while it was scrolled up (the scroll bar is red).
    pub new_since_scroll: bool,
    /// The text is a draft brought back and not yet edited: grey and slanted
    /// (`ChatScreen.formatChat`).
    pub draft: bool,
}

/// How the chat screen's chat was drawn: its lines, how many of them came after the
/// input's newest message, and the newest message then. While the chat is scrolled up,
/// each new line scrolls it up one more (`addMessageToDisplayQueue`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatShown {
    pub lines: usize,
    pub added: i32,
    pub newest: Option<Arc<ruthenium_sim::hud_text::ChatMessage>>,
}

/// One frame's HUD.
#[derive(Debug, Default)]
pub struct HudLayout {
    /// Drawn first, inverting what is behind it.
    pub crosshair: DrawList,
    /// Under the items.
    pub under: DrawList,
    pub items: Vec<HudItem>,
    /// Over the items.
    pub over: DrawList,
    texts: Vec<HudText>,
    /// How many of `texts` this frame uses: the rest keep their room for later.
    texts_used: usize,
    /// The faces in the player list.
    pub faces: Vec<HudFace>,
    /// The chat screen's chat, as last drawn.
    pub chat_shown: Option<ChatShown>,
    sorted: Vec<HudEffect>,
}

/// `Mth.clamp` for floats.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min { min } else { v.min(max) }
}

/// `ARGB.white(alpha)`.
fn white(alpha: f32) -> u32 {
    (mth::floor_f32(alpha * 255.0) as u32) << 24 | 0x00FF_FFFF
}

/// `Mth.hsvToRgb`, 0xRRGGBB.
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> u32 {
    let h = (hue * 6.0) as i32 % 6;
    let f = hue * 6.0 - h as f32;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - f * saturation);
    let t = value * (1.0 - (1.0 - f) * saturation);
    let [r, g, b] = match h {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    };
    let byte = |c: f32| ((c * 255.0) as i32).clamp(0, 255) as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

/// `Font.width` of plain text.
pub fn text_width(font: &Font, text: &str) -> i32 {
    let width: f32 = text.chars().map(|c| font.glyph(c).advance(false)).sum();
    width.ceil() as i32
}

/// `MobEffectInstance.compareTo`.
fn compare_effects(a: &HudEffect, b: &HudEffect) -> std::cmp::Ordering {
    const CUT_OFF: i32 = 32147;
    let colour = |e: &HudEffect| {
        usize::try_from(e.id)
            .ok()
            .and_then(|i| EFFECTS.get(i))
            .map_or(0, |e| e.2)
    };
    let infinite = |e: &HudEffect| e.duration == -1;
    if (a.duration <= CUT_OFF || b.duration <= CUT_OFF) && !(a.ambient && b.ambient) {
        a.ambient
            .cmp(&b.ambient)
            .then(infinite(a).cmp(&infinite(b)))
            .then(a.duration.cmp(&b.duration))
            .then(colour(a).cmp(&colour(b)))
    } else {
        a.ambient.cmp(&b.ambient).then(colour(a).cmp(&colour(b)))
    }
}

impl HudLayout {
    pub fn new() -> HudLayout {
        HudLayout::default()
    }

    pub fn clear(&mut self) {
        self.crosshair.clear();
        self.under.clear();
        self.items.clear();
        self.over.clear();
        self.texts_used = 0;
        self.faces.clear();
        self.chat_shown = None;
    }

    /// The frame's lines of text, each over so many of [`Self::over`] and under the rest.
    pub fn texts(&self) -> &[HudText] {
        &self.texts[..self.texts_used]
    }

    /// Lays out the frame's HUD (`Hud.extractRenderState`, for what there is of it).
    pub fn build(
        &mut self,
        sprites: &HudSprites,
        font: &Font,
        lang: &dyn Lang,
        view: &HudView<'_>,
        health: &mut HudHealth,
        memory: &mut HudMemory,
    ) {
        self.clear();
        memory.begin();
        self.crosshair(sprites, view);
        if !view.hud.spectator {
            self.hotbar(sprites, font, view);
        }
        if view.hud.survival {
            self.player_health(sprites, view, health);
            self.experience(sprites, font, view);
        }
        let words = &mut Words {
            sprites,
            font,
            lang,
            view,
            memory,
        };
        if !view.hud.spectator {
            self.selected_name(words);
        }
        self.effects(sprites, view);
        self.bosses(words);
        self.words(words);
    }

    /// `extractCrosshair`. A spectator's shows only on what it can open, which is not
    /// worked out yet: it has none.
    fn crosshair(&mut self, s: &HudSprites, v: &HudView<'_>) {
        let h = v.hud;
        if h.spectator || !v.options.crosshair {
            return;
        }
        let (t, list) = (s.texture, &mut self.crosshair);
        list.blit(
            &s.crosshair,
            t,
            (v.width - 15) / 2,
            (v.height - 15) / 2,
            15,
            15,
            WHITE,
        );
        if v.options.attack_indicator != AttackIndicator::Crosshair {
            return;
        }
        let strength = h.attack_strength;
        let full = h.target_alive && strength >= 1.0 && h.attack_delay > 5.0;
        let y = v.height / 2 - 7 + 16;
        let x = v.width / 2 - 8;
        if full {
            list.blit(&s.crosshair_full, t, x, y, 16, 16, WHITE);
        } else if strength < 1.0 {
            let progress = (strength * 17.0) as i32;
            list.blit(&s.crosshair_background, t, x, y, 16, 4, WHITE);
            list.blit_part(
                &s.crosshair_progress,
                t,
                [16, 4],
                [0, 0],
                x,
                y,
                progress,
                4,
                WHITE,
            );
        }
    }

    /// `extractItemHotbar`.
    fn hotbar(&mut self, s: &HudSprites, font: &Font, v: &HudView<'_>) {
        let h = v.hud;
        let t = s.texture;
        let centre = v.width / 2;
        let bottom = v.height;
        // The off hand is the left one, unless the main hand is.
        let off_left = !h.left_handed;
        let off = !h.slots[OFF_HAND].is_empty();
        self.under
            .blit(&s.hotbar, t, centre - 91, bottom - 22, 182, 22, WHITE);
        self.under.blit(
            &s.hotbar_selection,
            t,
            centre - 91 - 1 + h.selected * 20,
            bottom - 22 - 1,
            24,
            23,
            WHITE,
        );
        if off {
            if off_left {
                self.under.blit(
                    &s.hotbar_offhand_left,
                    t,
                    centre - 91 - 29,
                    bottom - 23,
                    29,
                    24,
                    WHITE,
                );
            } else {
                self.under.blit(
                    &s.hotbar_offhand_right,
                    t,
                    centre + 91,
                    bottom - 23,
                    29,
                    24,
                    WHITE,
                );
            }
        }
        let y = bottom - 16 - 3;
        for slot in 0..9 {
            self.slot(font, v, slot, centre - 90 + slot as i32 * 20 + 2, y);
        }
        if off {
            let x = if off_left {
                centre - 91 - 26
            } else {
                centre + 91 + 10
            };
            self.slot(font, v, OFF_HAND, x, y);
        }
        if v.options.attack_indicator == AttackIndicator::Hotbar && h.attack_strength < 1.0 {
            let y = bottom - 20;
            let x = if off_left {
                centre + 91 + 6
            } else {
                centre - 91 - 22
            };
            let progress = (h.attack_strength * 19.0) as i32;
            self.over
                .blit(&s.hotbar_attack_background, t, x, y, 18, 18, WHITE);
            self.over.blit_part(
                &s.hotbar_attack_progress,
                t,
                [18, 18],
                [0, 18 - progress],
                x,
                y + 18 - progress,
                18,
                progress,
                WHITE,
            );
        }
    }

    /// `extractSlot`: the item, and over it what `itemDecorations` adds: how worn
    /// it is, how long until it can be used again, and how many there are.
    fn slot(&mut self, font: &Font, v: &HudView<'_>, slot: usize, x: i32, y: i32) {
        let stack = &v.hud.slots[slot];
        if stack.is_empty() {
            return;
        }
        let pop = v.hud.pop[slot] as f32 - v.partial;
        let squeeze = (pop > 0.0).then(|| {
            let squeeze = 1.0 + pop / 5.0;
            [1.0 / squeeze, (squeeze + 1.0) / 2.0]
        });
        self.items.push(HudItem {
            slot,
            x,
            y,
            squeeze,
            z: 0.0,
        });
        let c = &stack.c;
        let damage = c.damage.unwrap_or(0);
        if c.damageable() && damage > 0 {
            let max = c.max_damage.unwrap_or(0);
            // Math.round, then the clamp.
            let width = mth::floor_f32(13.0 - damage as f32 * 13.0 / max as f32 + 0.5).clamp(0, 13);
            let left = ((max as f32 - damage as f32) / max as f32).max(0.0);
            let colour = BLACK | hsv_to_rgb(left / 3.0, 1.0, 1.0);
            self.over.fill(x + 2, y + 13, x + 15, y + 15, BLACK);
            self.over.fill(x + 2, y + 13, x + 2 + width, y + 14, colour);
        }
        let cooldown = v.hud.cooldowns[slot].percent(v.partial);
        if cooldown > 0.0 {
            let top = y + mth::floor_f32(16.0 * (1.0 - cooldown));
            let bottom = top + (16.0 * cooldown).ceil() as i32;
            self.over.fill(x, top, x + 16, bottom, COOLDOWN_COLOUR);
        }
        if stack.count != 1 {
            let mut digits = [0u8; 12];
            let text = decimal(stack.count, &mut digits);
            let width = text_width(font, text);
            self.text(
                format_args!("{text}"),
                x + 19 - 2 - width,
                y + 6 + 3,
                WHITE,
                true,
            );
        }
    }

    /// `extractPlayerHealth`: armour, hearts, food and air.
    fn player_health(&mut self, s: &HudSprites, v: &HudView<'_>, state: &mut HudHealth) {
        let h = v.hud;
        let ticks = i64::from(h.ticks);
        let current = h.health.ceil() as i32;
        let blink = state.blink_time > ticks && (state.blink_time - ticks) / 3 % 2 == 1;
        if current < state.last_health && h.damage_cooldown > 0 {
            state.last_health_time = v.millis;
            state.blink_time = ticks + 20;
        } else if current > state.last_health && h.damage_cooldown > 0 {
            state.last_health_time = v.millis;
            state.blink_time = ticks + 10;
        }
        if v.millis - state.last_health_time > 1000 {
            state.display_health = current;
            state.last_health_time = v.millis;
        }
        state.last_health = current;
        let old = state.display_health;
        let mut random = JavaRandom::new(i64::from(h.ticks.wrapping_mul(312_871)));
        let left = v.width / 2 - 91;
        let right = v.width / 2 + 91;
        let base = v.height - 39;
        let max_health = h.max_health.max(old.max(current) as f32);
        let absorption = h.absorption.ceil() as i32;
        let rows = ((max_health + absorption as f32) / 2.0 / 10.0).ceil() as i32;
        let row_height = (10 - (rows - 2)).max(3);
        let has = |effect: i32| h.effects.iter().any(|e| e.id == effect);
        let lifted = if has(REGENERATION) {
            h.ticks % (max_health + 5.0).ceil() as i32
        } else {
            -1
        };
        let t = s.texture;

        // extractArmor.
        if h.armor > 0 {
            let y = base - (rows - 1) * row_height - 10;
            for i in 0..10 {
                let sprite = match (i * 2 + 1).cmp(&h.armor) {
                    std::cmp::Ordering::Less => 2,
                    std::cmp::Ordering::Equal => 1,
                    std::cmp::Ordering::Greater => 0,
                };
                self.over
                    .blit(&s.armor[sprite], t, left + i * 8, y, 9, 9, WHITE);
            }
        }

        // extractHearts.
        let kind = if has(POISON) {
            Heart::Poisoned
        } else if has(WITHER) {
            Heart::Withered
        } else if h.fully_frozen {
            Heart::Frozen
        } else {
            Heart::Normal
        };
        let containers = (f64::from(max_health) / 2.0).ceil() as i32;
        let absorbing = (f64::from(absorption) / 2.0).ceil() as i32;
        for index in (0..containers + absorbing).rev() {
            let x = left + index % 10 * 8;
            let mut y = base - index / 10 * row_height;
            if current + absorption <= 4 {
                y += random.next_int(2);
            }
            if index < containers && index == lifted {
                y -= 2;
            }
            let mut heart = |kind: Heart, blinking: bool, half: bool| {
                self.over.blit(
                    s.heart(kind, h.hardcore, half, blinking),
                    t,
                    x,
                    y,
                    9,
                    9,
                    WHITE,
                );
            };
            heart(Heart::Container, blink, false);
            let halves = index * 2;
            if index >= containers {
                let of_absorption = halves - containers * 2;
                if of_absorption < absorption {
                    let kind = if kind == Heart::Withered {
                        kind
                    } else {
                        Heart::Absorbing
                    };
                    heart(kind, false, of_absorption + 1 == absorption);
                }
            }
            if blink && halves < old {
                heart(kind, true, halves + 1 == old);
            }
            if halves < current {
                heart(kind, false, halves + 1 == current);
            }
        }

        // extractFood. (A mount's hearts, which take its place, are not drawn yet.)
        let hungry = usize::from(has(HUNGER));
        for i in 0..10 {
            let mut y = base;
            if h.saturation <= 0.0 && h.ticks % (h.food * 3 + 1) == 0 {
                y += random.next_int(3) - 1;
            }
            let x = right - i * 8 - 9;
            self.over.blit(&s.food[hungry][0], t, x, y, 9, 9, WHITE);
            match (i * 2 + 1).cmp(&h.food) {
                std::cmp::Ordering::Less => {
                    self.over.blit(&s.food[hungry][2], t, x, y, 9, 9, WHITE)
                }
                std::cmp::Ordering::Equal => {
                    self.over.blit(&s.food[hungry][1], t, x, y, 9, 9, WHITE)
                }
                std::cmp::Ordering::Greater => {}
            }
        }

        // extractAirBubbles: a row above the food.
        let max_air = h.max_air;
        let air = h.air.clamp(0, max_air);
        if h.eye_in_water || air < max_air {
            let y = base - 10;
            let bubble =
                |offset: i32| ((((air + offset) * 10) as f32) / max_air as f32).ceil() as i32;
            let full = bubble(-2);
            let popping = bubble(0);
            let delay = i32::from(air != 0 && h.eye_in_water);
            let empty = 10 - bubble(delay);
            for i in 1..=10 {
                let x = right - (i - 1) * 8 - 9;
                if i <= full {
                    self.over.blit(&s.air[0], t, x, y, 9, 9, WHITE);
                } else if full != popping && i == popping && h.eye_in_water {
                    self.over.blit(&s.air[1], t, x, y, 9, 9, WHITE);
                } else if i > 10 - empty {
                    let wobble = if empty == 10 && h.ticks % 2 == 0 {
                        random.next_int(2)
                    } else {
                        0
                    };
                    self.over.blit(&s.air[2], t, x, y + wobble, 9, 9, WHITE);
                }
            }
        }
    }

    /// `ExperienceBar.extractBackground` and `ContextualBar.extractExperienceLevel`.
    fn experience(&mut self, s: &HudSprites, font: &Font, v: &HudView<'_>) {
        let h = v.hud;
        let t = s.texture;
        let left = (v.width - 182) / 2;
        let top = v.height - 24 - 5;
        if h.experience_needed() > 0 {
            let progress = (h.experience_progress * 183.0) as i32;
            self.over
                .blit(&s.experience_background, t, left, top, 182, 5, WHITE);
            if progress > 0 {
                self.over.blit_part(
                    &s.experience_progress,
                    t,
                    [182, 5],
                    [0, 0],
                    left,
                    top,
                    progress,
                    5,
                    WHITE,
                );
            }
        }
        if h.experience_level > 0 {
            // "gui.experience.level" is the number alone.
            let mut digits = [0u8; 12];
            let text = decimal(h.experience_level, &mut digits);
            let x = (v.width - text_width(font, text)) / 2;
            let y = v.height - 24 - 9 - 2;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                self.text(format_args!("{text}"), x + dx, y + dy, BLACK, false);
            }
            self.text(format_args!("{text}"), x, y, LEVEL_COLOUR, false);
        }
    }

    /// `extractEffects`: the good ones in a row from the top right corner, the others
    /// in a row below.
    fn effects(&mut self, s: &HudSprites, v: &HudView<'_>) {
        let mut sorted = std::mem::take(&mut self.sorted);
        sorted.clear();
        sorted.extend_from_slice(&v.hud.effects);
        // Ordering.natural().reverse().sortedCopy: stable.
        sorted.sort_by(|a, b| compare_effects(b, a));
        let t = s.texture;
        let (mut good, mut bad) = (0, 0);
        for e in sorted.iter().filter(|e| e.show_icon) {
            let known = usize::try_from(e.id).ok().filter(|i| *i < EFFECTS.len());
            let mut x = v.width;
            let mut y = 1;
            if known.is_some_and(|i| EFFECTS[i].1) {
                good += 1;
                x -= 25 * good;
            } else {
                bad += 1;
                x -= 25 * bad;
                y += 26;
            }
            let mut alpha = 1.0;
            if e.ambient {
                self.over
                    .blit(&s.effect_background_ambient, t, x, y, 24, 24, WHITE);
            } else {
                self.over.blit(&s.effect_background, t, x, y, 24, 24, WHITE);
                if e.duration != -1 && e.duration <= 200 {
                    let left = e.duration;
                    let used_seconds = 10 - left / 20;
                    alpha = clamp(left as f32 / 10.0 / 5.0 * 0.5, 0.0, 0.5)
                        + mth::cos(f64::from(left as f32 * std::f32::consts::PI / 5.0))
                            * clamp(used_seconds as f32 / 10.0 * 0.25, 0.0, 0.25);
                    alpha = clamp(alpha, 0.0, 1.0);
                }
            }
            let sprite = known.map_or(&s.missing, |i| &s.effects[i]);
            self.over
                .blit(sprite, t, x + 3, y + 3, 18, 18, white(alpha));
        }
        self.sorted = sorted;
    }
}

/// A number's decimal digits, written without asking for memory.
fn decimal(n: i32, out: &mut [u8; 12]) -> &str {
    use std::io::Write;
    let mut rest = &mut out[..];
    let _ = write!(rest, "{n}");
    let left = rest.len();
    let len = out.len() - left;
    std::str::from_utf8(&out[..len]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ruthenium_assets::font::Glyph;
    use ruthenium_assets::meta::GuiScaling;
    use ruthenium_sim::hud::Cooldown;
    use ruthenium_sim::item::{Components, Stack};

    use super::*;
    use crate::draw::Quad;

    /// Sprites that can be told apart: each one's coordinates begin at its number.
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

        /// What a list draws: each sprite's name (nothing for a filled quad) and where.
        fn drawn(&self, list: &DrawList) -> Vec<(String, [i32; 4])> {
            list.quads()
                .iter()
                .map(|q| (self.name(q), place(q)))
                .collect()
        }

        fn name(&self, q: &Quad) -> String {
            if q.texture == crate::NO_TEXTURE {
                return String::new();
            }
            self.0.borrow()[q.uv[0].floor() as usize].clone()
        }
    }

    /// Left, top, width, height.
    fn place(q: &Quad) -> [i32; 4] {
        let [x0, y0, x1, y1] = q.rect.map(|v| v as i32);
        [x0, y0, x1 - x0, y1 - y0]
    }

    /// Every digit is 6 wide, as the game's.
    fn font() -> Font {
        Font::of_glyphs(('0'..='9').map(|c| {
            (
                c,
                Glyph {
                    advance: 6.0,
                    ..Glyph::MISSING
                },
            )
        }))
    }

    fn survivor() -> HudRecord {
        HudRecord {
            survival: true,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            attack_strength: 1.0,
            attack_delay: 12.5,
            ..HudRecord::default()
        }
    }

    struct Laid {
        names: Names,
        layout: HudLayout,
    }

    impl Laid {
        fn over(&self) -> Vec<(String, [i32; 4])> {
            self.names.drawn(&self.layout.over)
        }

        fn of(&self, prefix: &str) -> Vec<(String, [i32; 4])> {
            self.over()
                .into_iter()
                .filter(|(name, _)| name.starts_with(prefix))
                .collect()
        }
    }

    /// The HUD of `hud` on a GUI of 427 by 240.
    fn lay_with(hud: &HudRecord, health: &mut HudHealth, millis: i64, options: HudOptions) -> Laid {
        let names = Names(RefCell::new(Vec::new()));
        let sprites = names.sprites();
        let mut layout = HudLayout::new();
        let view = HudView {
            hud,
            partial: 0.0,
            width: 427,
            height: 240,
            millis,
            now: Instant::now(),
            options,
            chat: None,
        };
        let mut memory = HudMemory::default();
        layout.build(&sprites, &font(), &(), &view, health, &mut memory);
        Laid { names, layout }
    }

    fn lay(hud: &HudRecord) -> Laid {
        // The hearts have shown this health for a while.
        let mut health = HudHealth::default();
        lay_with(hud, &mut health, 5000, HudOptions::default())
    }

    fn stack(count: i32, c: Components) -> Stack {
        Stack {
            identity: 1,
            item: 1,
            count,
            c: std::sync::Arc::new(c),
            ..Stack::empty()
        }
    }

    #[test]
    fn hearts_and_food_stand_over_the_hotbars_ends() {
        let l = lay(&survivor());
        let hearts = l.of("hud/heart/");
        // Ten containers with a full heart in each, the last first.
        assert_eq!(hearts.len(), 20);
        assert_eq!(
            hearts[0],
            ("hud/heart/container".into(), [122 + 72, 201, 9, 9])
        );
        assert_eq!(hearts[1], ("hud/heart/full".into(), [122 + 72, 201, 9, 9]));
        assert_eq!(hearts[19], ("hud/heart/full".into(), [122, 201, 9, 9]));
        let food = l.of("hud/food");
        assert_eq!(food.len(), 20);
        assert_eq!(food[0], ("hud/food_empty".into(), [304 - 9, 201, 9, 9]));
        assert_eq!(food[1], ("hud/food_full".into(), [304 - 9, 201, 9, 9]));
        assert_eq!(food[18].1, [304 - 72 - 9, 201, 9, 9]);
        // No armour worn, no air lost: neither row is there.
        assert!(l.of("hud/armor").is_empty() && l.of("hud/air").is_empty());
        // Outside survival there are no bars at all.
        let creative = HudRecord {
            survival: false,
            ..survivor()
        };
        assert!(lay(&creative).over().is_empty());
    }

    #[test]
    fn half_hearts_halves_of_food_and_armour() {
        let hud = HudRecord {
            health: 12.5,
            food: 7,
            armor: 5,
            hardcore: true,
            ..survivor()
        };
        let l = lay(&hud);
        let hearts = l.of("hud/heart/");
        // 13 halves: six full hearts and a half one in the seventh container.
        let filled: Vec<_> = hearts
            .iter()
            .filter(|(n, _)| !n.contains("container"))
            .collect();
        assert_eq!(filled.len(), 7);
        assert_eq!(
            *filled[0],
            ("hud/heart/hardcore_half".into(), [122 + 48, 201, 9, 9])
        );
        assert!(
            filled[1..]
                .iter()
                .all(|(n, _)| n == "hud/heart/hardcore_full")
        );
        assert!(
            hearts
                .iter()
                .any(|(n, _)| n == "hud/heart/container_hardcore")
        );
        let food: Vec<String> = l.of("hud/food").into_iter().map(|(n, _)| n).collect();
        assert_eq!(food.iter().filter(|n| *n == "hud/food_full").count(), 3);
        assert_eq!(food.iter().filter(|n| *n == "hud/food_half").count(), 1);
        // From the right: three full, the half, then empty ones alone.
        assert_eq!(food[7], "hud/food_half");
        assert_eq!(food[8..10], ["hud/food_empty", "hud/food_empty"]);
        let armor = l.of("hud/armor");
        assert_eq!(armor.len(), 10);
        assert_eq!(armor[0], ("hud/armor_full".into(), [122, 191, 9, 9]));
        assert_eq!(armor[2].0, "hud/armor_half");
        assert_eq!(armor[3], ("hud/armor_empty".into(), [146, 191, 9, 9]));
    }

    #[test]
    fn more_hearts_stand_in_rows_that_close_up() {
        // 40 health and 8 of absorption: 24 containers in three rows, nine apart.
        let hud = HudRecord {
            health: 40.0,
            max_health: 40.0,
            absorption: 8.0,
            armor: 1,
            ..survivor()
        };
        let l = lay(&hud);
        let hearts = l.of("hud/heart/");
        let containers: Vec<_> = hearts
            .iter()
            .filter(|(n, _)| n == "hud/heart/container")
            .collect();
        assert_eq!(containers.len(), 24);
        assert_eq!(containers[0].1, [122 + 24, 201 - 18, 9, 9]);
        assert_eq!(containers[23].1, [122, 201, 9, 9]);
        let absorbing: Vec<_> = hearts
            .iter()
            .filter(|(n, _)| n == "hud/heart/absorbing_full")
            .collect();
        assert_eq!(absorbing.len(), 4);
        // The armour stands over the top row.
        assert_eq!(l.of("hud/armor")[0].1, [122, 201 - 18 - 10, 9, 9]);
        // Wither blackens the absorbed ones too; poison does not.
        let withered = HudRecord {
            effects: vec![effect(WITHER, 100)],
            ..hud.clone()
        };
        let names: Vec<String> = lay(&withered)
            .of("hud/heart/")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/withered_full")
                .count(),
            24
        );
        let poisoned = HudRecord {
            effects: vec![effect(POISON, 100)],
            ..hud
        };
        let names: Vec<String> = lay(&poisoned)
            .of("hud/heart/")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/poisoned_full")
                .count(),
            20
        );
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/absorbing_full")
                .count(),
            4
        );
    }

    fn effect(id: i32, duration: i32) -> HudEffect {
        HudEffect {
            id,
            amplifier: 0,
            duration,
            ambient: false,
            show_icon: true,
        }
    }

    #[test]
    fn low_health_hunger_and_no_air_shake_as_the_games_random_says() {
        // What java.util.Random draws after setSeed(14 * 312871): ten of nextInt(2), ten
        // of nextInt(3) - 1, ten of nextInt(2).
        const HEARTS: [i32; 10] = [1, 1, 1, 0, 1, 1, 0, 0, 0, 1];
        const FOOD: [i32; 10] = [0, 1, -1, 1, 1, -1, 0, 1, -1, -1];
        const AIR: [i32; 10] = [0, 0, 0, 0, 0, 0, 0, 1, 1, 1];
        let hud = HudRecord {
            ticks: 14,
            health: 3.0,
            food: 2,
            saturation: 0.0,
            air: 0,
            eye_in_water: true,
            ..survivor()
        };
        let l = lay(&hud);
        let containers: Vec<i32> = l
            .of("hud/heart/container")
            .iter()
            .map(|(_, at)| at[1] - 201)
            .collect();
        assert_eq!(containers, HEARTS);
        let food: Vec<i32> = l
            .of("hud/food_empty")
            .iter()
            .map(|(_, at)| at[1] - 201)
            .collect();
        assert_eq!(food, FOOD);
        let air = l.of("hud/air");
        assert!(air.iter().all(|(n, _)| n == "hud/air_empty"));
        let air: Vec<i32> = air.iter().map(|(_, at)| at[1] - 191).collect();
        assert_eq!(air, AIR);
        // A tick later nothing of the food and the air moves: 15 is neither a multiple
        // of 7 nor even.
        let l = lay(&HudRecord { ticks: 15, ..hud });
        assert!(l.of("hud/food").iter().all(|(_, at)| at[1] == 201));
        assert!(l.of("hud/air").iter().all(|(_, at)| at[1] == 191));
    }

    #[test]
    fn air_runs_out_bubble_by_bubble() {
        let air = |air: i32, eye_in_water: bool| {
            let hud = HudRecord {
                air,
                eye_in_water,
                ..survivor()
            };
            let names: Vec<String> = lay(&hud)
                .of("hud/air")
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            (
                names.iter().filter(|n| *n == "hud/air").count(),
                names.iter().filter(|n| *n == "hud/air_bursting").count(),
                names.iter().filter(|n| *n == "hud/air_empty").count(),
            )
        };
        // Full air under water shows all ten; out of the water, none.
        assert_eq!(air(300, true), (10, 0, 0));
        assert_eq!(air(300, false), (0, 0, 0));
        // A bubble bursts for two ticks before it is gone, and is empty a tick later.
        assert_eq!(air(272, true), (9, 1, 0));
        assert_eq!(air(271, true), (9, 1, 0));
        assert_eq!(air(270, true), (9, 0, 0));
        assert_eq!(air(269, true), (9, 0, 1));
        // Coming back up, nothing bursts.
        assert_eq!(air(271, false), (9, 0, 0));
        assert_eq!(air(0, true), (0, 0, 10));
        // The row stands over the food.
        let hud = HudRecord {
            air: 150,
            eye_in_water: true,
            ..survivor()
        };
        assert_eq!(lay(&hud).of("hud/air")[0].1, [295, 191, 9, 9]);
    }

    #[test]
    fn hearts_blink_at_a_hurt_whose_health_came_before_set_health() {
        // Live on vanilla and Paper (2026-11-28): after `/damage` (and a player's hit)
        // the lower health comes first in the player's entity data, then the damage
        // event, then a `set_health` that changes nothing; only the damage event's
        // cooldown lets the hearts see the loss (Hud.java:771).
        use ruthenium_sim::hud::HudState;
        use ruthenium_sim::player::LocalPlayer;
        let mut p = LocalPlayer::new();
        p.hurt_to(20.0);
        let mut state = HudState::default();
        let mut health = HudHealth::default();
        let mut record = HudRecord::default();
        let mut show = |p: &LocalPlayer, state: &mut HudState, millis: i64| {
            state.tick(p);
            record.fill(state, p, false, false, false);
            record.survival = true;
            lay_with(&record, &mut health, millis, HudOptions::default())
                .of("hud/heart/container")
                .iter()
                .any(|(n, _)| n.ends_with("_blinking"))
        };
        assert!(!show(&p, &mut state, 10_000));
        p.health = 17.0;
        p.handle_damage_event();
        p.hurt_to(17.0);
        let blinks: Vec<bool> = (0..20)
            .map(|i| show(&p, &mut state, 10_050 + i * 50))
            .collect();
        assert!(blinks.iter().any(|b| *b), "{blinks:?}");
    }

    #[test]
    fn hearts_blink_when_health_changes_and_show_what_was_lost() {
        let mut health = HudHealth::default();
        let options = HudOptions::default();
        let at = |ticks: i32, value: f32, cooldown: i32| HudRecord {
            ticks,
            health: value,
            damage_cooldown: cooldown,
            ..survivor()
        };
        let blinking = |l: &Laid| {
            l.of("hud/heart/")
                .iter()
                .any(|(n, _)| n.ends_with("_blinking"))
        };
        // Settled at 20.
        let l = lay_with(&at(100, 20.0, 0), &mut health, 10_000, options);
        assert!(!blinking(&l));
        // Hurt to 14 at tick 100: the blinking is of the 20 ticks from then, three ticks
        // on and three off, counted back from their end.
        let l = lay_with(&at(100, 14.0, 20), &mut health, 10_050, options);
        assert!(!blinking(&l));
        let on: Vec<bool> = (101..=121)
            .map(|tick| {
                let l = lay_with(&at(tick, 14.0, 10), &mut health, 10_100, options);
                blinking(&l)
            })
            .collect();
        let (t, f) = (true, false);
        assert_eq!(
            on,
            [
                f, f, t, t, t, f, f, f, t, t, t, f, f, f, t, t, t, f, f, f, f
            ]
        );
        // While it blinks, the hearts lost show as blinking ones behind what is left: for
        // a second from the change.
        let l = lay_with(&at(103, 14.0, 10), &mut health, 10_900, options);
        let names: Vec<String> = l.of("hud/heart/").into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/full_blinking")
                .count(),
            10
        );
        assert_eq!(names.iter().filter(|n| *n == "hud/heart/full").count(), 7);
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/container_blinking")
                .count(),
            10
        );
        // A second after the health last changed, what was lost is forgotten.
        let l = lay_with(&at(103, 14.0, 10), &mut health, 11_051, options);
        let names: Vec<String> = l.of("hud/heart/").into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names
                .iter()
                .filter(|n| *n == "hud/heart/full_blinking")
                .count(),
            7
        );
        // Healed with no cooldown running (the first health a server sends): no blink.
        let mut fresh = HudHealth::default();
        lay_with(&at(5, 20.0, 0), &mut fresh, 10_000, options);
        assert_eq!(fresh.blink_time, 0);
        // Regeneration lifts one heart after the other.
        let regen = HudRecord {
            ticks: 53,
            effects: vec![effect(REGENERATION, 100)],
            ..survivor()
        };
        let lifted: Vec<_> = lay(&regen)
            .of("hud/heart/")
            .into_iter()
            .filter(|(_, at)| at[1] == 199)
            .collect();
        // 53 % 25 = 3: the fourth heart, its container and what is in it.
        assert_eq!(lifted.len(), 2);
        assert_eq!(lifted[0].1[0], 122 + 24);
    }

    #[test]
    fn the_experience_bar_fills_and_its_level_stands_over_it() {
        let hud = HudRecord {
            experience_progress: 0.5,
            experience_level: 30,
            ..survivor()
        };
        let l = lay(&hud);
        let bar = l.of("hud/experience");
        assert_eq!(
            bar[0],
            ("hud/experience_bar_background".into(), [122, 211, 182, 5])
        );
        assert_eq!(
            bar[1],
            ("hud/experience_bar_progress".into(), [122, 211, 91, 5])
        );
        // "30" is 12 wide: black on four sides, then green.
        let texts = l.layout.texts();
        assert_eq!(texts.len(), 5);
        let places: Vec<(i32, i32, u32)> = texts
            .iter()
            .map(|t| (t.x as i32, t.y as i32, t.color))
            .collect();
        assert_eq!(
            places,
            [
                (208, 205, BLACK),
                (206, 205, BLACK),
                (207, 206, BLACK),
                (207, 204, BLACK),
                (207, 205, 0xFF80_FF20)
            ]
        );
        assert!(texts.iter().all(|t| t.text == "30" && !t.shadow));
        // Almost a level: all 182 units of the bar.
        let nearly = HudRecord {
            experience_progress: 0.999,
            ..survivor()
        };
        assert_eq!(lay(&nearly).of("hud/experience")[1].1, [122, 211, 182, 5]);
        // No level and no progress: the empty bar alone.
        let l = lay(&survivor());
        assert_eq!(l.of("hud/experience").len(), 1);
        assert!(l.layout.texts().is_empty());
    }

    #[test]
    fn the_hotbar_holds_its_items_and_what_is_said_of_them() {
        let worn = Components {
            max_damage: Some(100),
            damage: Some(25),
            ..Components::default()
        };
        let mut hud = HudRecord {
            selected: 3,
            ..survivor()
        };
        hud.slots[0] = stack(64, Components::default());
        hud.slots[3] = stack(1, worn);
        hud.slots[5] = stack(
            1,
            Components {
                max_damage: Some(100),
                damage: Some(75),
                ..Components::default()
            },
        );
        hud.slots[8] = stack(1, Components::default());
        hud.cooldowns[8] = Cooldown {
            left: 5.0,
            of: 20.0,
        };
        hud.pop[0] = 5;
        hud.slots[OFF_HAND] = stack(1, Components::default());
        let l = lay(&hud);
        assert_eq!(
            l.names.drawn(&l.layout.under),
            [
                ("hud/hotbar".into(), [122, 218, 182, 22]),
                ("hud/hotbar_selection".into(), [121 + 60, 217, 24, 23]),
                ("hud/hotbar_offhand_left".into(), [93, 217, 29, 24]),
            ]
        );
        let items: Vec<(usize, i32, i32)> =
            l.layout.items.iter().map(|i| (i.slot, i.x, i.y)).collect();
        assert_eq!(
            items,
            [
                (0, 125, 221),
                (3, 185, 221),
                (5, 225, 221),
                (8, 285, 221),
                (OFF_HAND, 96, 221)
            ]
        );
        // A stack that just grew: squeezed to half its width, half again as tall.
        assert_eq!(l.layout.items[0].squeeze, Some([0.5, 1.5]));
        assert_eq!(l.layout.items[1].squeeze, None);
        // 64, right-aligned in its slot, with a shadow.
        let texts = l.layout.texts();
        assert_eq!(texts.len(), 1);
        assert_eq!(
            (texts[0].text.as_str(), texts[0].x, texts[0].y),
            ("64", (125 + 17 - 12) as f32, 230.0)
        );
        assert!(texts[0].shadow);
        // A quarter worn: a black bar with ten of thirteen units lit, between yellow and
        // green; and the cooldown's quarter over the slot's bottom.
        let fills: Vec<([i32; 4], u32)> = l
            .layout
            .over
            .quads()
            .iter()
            .filter(|q| q.texture == crate::NO_TEXTURE)
            .map(|q| (place(q), q.color))
            .collect();
        assert_eq!(
            fills,
            [
                ([187, 234, 13, 2], BLACK),
                ([187, 234, 10, 1], 0xFF7F_FF00),
                // Three quarters worn: three units, between red and yellow.
                ([227, 234, 13, 2], BLACK),
                ([227, 234, 3, 1], 0xFFFF_7F00),
                ([285, 233, 16, 4], 0x7FFF_FFFF),
            ]
        );
        // A left-handed player holds its other hand's item on the right.
        hud.left_handed = true;
        let l = lay(&hud);
        assert_eq!(
            l.names.drawn(&l.layout.under)[2],
            ("hud/hotbar_offhand_right".into(), [304, 217, 29, 24])
        );
        assert_eq!(l.layout.items[4].x, 314);
        // A spectator has no hotbar.
        hud.spectator = true;
        let l = lay(&hud);
        assert!(l.layout.under.quads().is_empty() && l.layout.items.is_empty());
        assert!(l.layout.crosshair.quads().is_empty());
    }

    #[test]
    fn the_attack_indicator_fills_under_the_crosshair_or_beside_the_hotbar() {
        let mut health = HudHealth::default();
        let crosshair = |hud: &HudRecord, options: HudOptions| {
            let l = lay_with(hud, &mut HudHealth::default(), 5000, options);
            l.names.drawn(&l.layout.crosshair)
        };
        let options = HudOptions::default();
        // At rest: the crosshair alone, in the middle.
        assert_eq!(
            crosshair(&survivor(), options),
            [("hud/crosshair".into(), [206, 112, 15, 15])]
        );
        // Half charged: the bar's ground and eight of its sixteen units.
        let charging = HudRecord {
            attack_strength: 0.5,
            ..survivor()
        };
        assert_eq!(
            crosshair(&charging, options)[1..],
            [
                (
                    "hud/crosshair_attack_indicator_background".into(),
                    [205, 129, 16, 4]
                ),
                (
                    "hud/crosshair_attack_indicator_progress".into(),
                    [205, 129, 8, 4]
                ),
            ]
        );
        // Almost charged: sixteen of seventeen steps, the bar full but for the last.
        let nearly = HudRecord {
            attack_strength: 0.95,
            ..survivor()
        };
        assert_eq!(crosshair(&nearly, options)[2].1, [205, 129, 16, 4]);
        // Charged, on something alive, with a weapon slower than five ticks.
        let aimed = HudRecord {
            target_alive: true,
            ..survivor()
        };
        assert_eq!(
            crosshair(&aimed, options)[1],
            (
                "hud/crosshair_attack_indicator_full".into(),
                [205, 129, 16, 16]
            )
        );
        let fast = HudRecord {
            attack_delay: 5.0,
            ..aimed.clone()
        };
        assert_eq!(crosshair(&fast, options).len(), 1);
        // Beside the hotbar instead: on the main hand's side, filling from below.
        let hotbar = HudOptions {
            attack_indicator: AttackIndicator::Hotbar,
            ..options
        };
        assert_eq!(crosshair(&charging, hotbar).len(), 1);
        let l = lay_with(&charging, &mut health, 5000, hotbar);
        assert_eq!(
            l.of("hud/hotbar_attack"),
            [
                (
                    "hud/hotbar_attack_indicator_background".into(),
                    [310, 220, 18, 18]
                ),
                (
                    "hud/hotbar_attack_indicator_progress".into(),
                    [310, 229, 18, 9]
                ),
            ]
        );
        let progress = l
            .layout
            .over
            .quads()
            .iter()
            .find(|q| l.names.name(q) == "hud/hotbar_attack_indicator_progress")
            .unwrap();
        // The lower half of the sprite.
        assert_eq!(progress.uv[1], 0.5);
        let off = HudOptions {
            attack_indicator: AttackIndicator::Off,
            ..options
        };
        assert_eq!(crosshair(&charging, off).len(), 1);
        let none = HudOptions {
            crosshair: false,
            ..options
        };
        assert!(crosshair(&charging, none).is_empty());
    }

    #[test]
    fn effects_line_up_from_the_corner_good_over_bad() {
        let mut ambient = effect(15, 400);
        ambient.ambient = true;
        let mut hidden = effect(4, 900);
        hidden.show_icon = false;
        let hud = HudRecord {
            effects: vec![
                effect(0, 600),  // speed
                effect(18, 300), // poison
                hidden,
                effect(7, -1),  // jump boost, for ever
                ambient,        // night vision from a beacon
                effect(1, 100), // slowness, about to end
                effect(17, 30), // weakness, all but gone
            ],
            ..survivor()
        };
        let l = lay(&hud);
        let icons: Vec<(String, [i32; 4])> = l
            .over()
            .into_iter()
            .filter(|(n, _)| n.starts_with("mob_effect/") || n.starts_with("hud/effect"))
            .collect();
        // The last to end first: those from a beacon, then those that never end, then by
        // the time left. The good in the top row, the others below.
        let order: Vec<&str> = icons
            .iter()
            .filter(|(n, _)| n.starts_with("mob_effect/"))
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            order,
            [
                "mob_effect/night_vision",
                "mob_effect/jump_boost",
                "mob_effect/speed",
                "mob_effect/poison",
                "mob_effect/slowness",
                "mob_effect/weakness"
            ]
        );
        assert_eq!(
            icons[0],
            ("hud/effect_background_ambient".into(), [402, 1, 24, 24])
        );
        assert_eq!(
            icons[1],
            ("mob_effect/night_vision".into(), [405, 4, 18, 18])
        );
        assert_eq!(icons[2], ("hud/effect_background".into(), [377, 1, 24, 24]));
        assert_eq!(icons[4].1, [352, 1, 24, 24]);
        assert_eq!(
            icons[6],
            ("hud/effect_background".into(), [402, 27, 24, 24])
        );
        assert_eq!(icons[8].1, [377, 27, 24, 24]);
        // An effect about to end flashes: 100 ticks left is 0.5 + cos(20 pi) * 0.125.
        let alpha = |name: &str| {
            l.layout
                .over
                .quads()
                .iter()
                .find(|q| l.names.name(q) == name)
                .unwrap()
                .color
                >> 24
        };
        assert_eq!(alpha("mob_effect/speed"), 255);
        assert_eq!(alpha("mob_effect/slowness"), 159);
        // 30 ticks left: 0.3 + cos(6 pi) * 0.225.
        assert_eq!(alpha("mob_effect/weakness"), 133);
        // Effects show in every game mode.
        let creative = HudRecord {
            survival: false,
            ..hud
        };
        assert_eq!(lay(&creative).over().len(), 12);
    }

    #[test]
    fn colours_are_the_games() {
        // Mth.hsvToRgb(h, 1, 1) for a full, half and empty bar.
        assert_eq!(hsv_to_rgb(1.0 / 3.0, 1.0, 1.0), 0x00FF00);
        assert_eq!(hsv_to_rgb(0.5 / 3.0, 1.0, 1.0), 0xFFFF00);
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), 0xFF0000);
        assert_eq!(white(1.0), 0xFFFF_FFFF);
        assert_eq!(white(0.5), 0x7FFF_FFFF);
        assert_eq!(white(0.0), 0x00FF_FFFF);
        let mut digits = [0u8; 12];
        assert_eq!(decimal(-2_147_483_648, &mut digits), "-2147483648");
        assert_eq!(decimal(64, &mut digits), "64");
    }
}
