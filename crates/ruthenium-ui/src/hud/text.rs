//! The HUD's words (`Hud`, `ChatComponent`, `BossHealthOverlay`, `PlayerTabOverlay`), laid
//! out as vanilla lays them out: the selected item's name, the boss bars, the sidebar, the
//! message over the hotbar, the titles, the chat and the player list; and the chat screen
//! over them (`ChatScreen`, its `EditBox`): the chat in the foreground, scrolled, and what
//! is being typed.
//!
//! Not yet: text that runs from right to left, fonts other than the game's usual one, and
//! pictures within text; the selection drawn inverted (it is see-through blue), and the
//! tooltips and the cursor over the chat's text.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_assets::Font;
use ruthenium_javamath::mth;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::text::{Lang, Style, StyledText};
use ruthenium_sim::avatar::SkinSource;
use ruthenium_sim::hud_text::{ChatMessage, ListScores, ListedPlayer};

use super::{
    BLACK, ChatInput, ChatShown, Heart, HudLayout, HudSprites, HudView, WHITE, hsv_to_rgb,
    text_width, white,
};

/// Room between lines' tops.
const LINE: i32 = 9;
/// The chat's options, as they are by default: its width in GUI units (`chatWidth`), the
/// lines it shows while it is not typed in (`chatHeightUnfocused` and `chatLineSpacing`),
/// and how much of what is behind it its ground covers (`textBackgroundOpacity`).
const CHAT_WIDTH: i32 = 320;
const CHAT_LINES: usize = 10;
const CHAT_GROUND: f32 = 0.5;
/// Lines the chat keeps (`trimmedMessages`).
const CHAT_KEPT: usize = 100;
/// Lines the chat shows while it is typed in (`chatHeightFocused` at its default, 180
/// units, over 9 a line).
const CHAT_FOCUSED_LINES: i32 = 20;
/// The chat screen's edit box: the ground behind it (`getBackgroundColor` with the
/// background only for the chat), its text (`EditBox.DEFAULT_TEXT_COLOR`), the
/// selection (`textHighlight`'s blue, drawn see-through) and its length in UTF-16 units.
const CHAT_INPUT_GROUND: u32 = 0x8000_0000;
const CHAT_INPUT_TEXT: u32 = 0xFFE0_E0E0;
const CHAT_INPUT_SELECTION: u32 = 0x8000_00FF;
/// A draft's text (`ChatFormatting.GRAY`), slanted.
const CHAT_DRAFT: u32 = 0xAA_AAAA;
const CHAT_INPUT_MAX: usize = 256;
/// The room a changed message's icon takes.
const CHAT_ICON: i32 = 9 + 4 + 2;
/// What was laid out is forgotten after this many frames without it.
const KEPT_FRAMES: u64 = 256;
const LIST_GROUND: u32 = 0x8000_0000;
const LIST_SLOT: u32 = 0x20FF_FFFF;
const LIST_SPECTATOR: u32 = 0x90FF_FFFF;
const LIST_ROWS: i32 = 20;

/// Characters with their styles, on one line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextLine {
    pub chars: Vec<(char, Style)>,
    /// `Font.width`.
    pub width: i32,
}

impl TextLine {
    fn new(chars: Vec<(char, Style)>, font: &Font) -> TextLine {
        let width: f32 = chars
            .iter()
            .map(|(c, s)| font.glyph(*c).advance(s.bold))
            .sum();
        TextLine {
            chars,
            width: width.ceil() as i32,
        }
    }
}

/// `Font.plainSubstrByWidth`: the longest start of `s` no wider than `width`.
pub(super) fn head_by_width<'a>(s: &'a str, width: i32, font: &Font) -> &'a str {
    let mut used = 0.0f32;
    for (i, c) in s.char_indices() {
        used += font.glyph(c).advance(false);
        if used > width as f32 {
            return &s[..i];
        }
    }
    s
}

/// The same from the end: the longest end of `s` no wider than `width`.
fn tail_by_width<'a>(s: &'a str, width: i32, font: &Font) -> &'a str {
    let mut used = 0.0f32;
    for (i, c) in s.char_indices().rev() {
        used += font.glyph(c).advance(false);
        if used > width as f32 {
            return &s[i + c.len_utf8()..];
        }
    }
    s
}

/// A component's characters.
fn characters(tag: &Tag, lang: &dyn Lang) -> Vec<(char, Style)> {
    StyledText::from_nbt_in(tag, lang)
        .spans
        .iter()
        .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
        .collect()
}

/// `StringSplitter.splitLines`: lines at most `width` wide, broken at spaces where there
/// are any, and whether each goes on from the one before it.
fn split(chars: &[(char, Style)], width: i32, font: &Font) -> Vec<(Vec<(char, Style)>, bool)> {
    let max = width.max(1) as f32;
    let mut lines = Vec::new();
    let mut rest = chars;
    let (mut wrapped, mut new_line) = (false, false);
    'lines: loop {
        let (mut taken, mut wide) = (0.0f32, false);
        let mut space = None;
        for (i, (c, style)) in rest.iter().enumerate() {
            let end = if *c == '\n' {
                Some(i)
            } else {
                if *c == ' ' {
                    space = Some(i);
                }
                let advance = font.glyph(*c).advance(style.bold);
                taken += advance;
                // What has no width never begins an overflow, and one character
                // always fits.
                let over = wide && taken > max;
                wide |= advance != 0.0;
                over.then(|| space.unwrap_or(i))
            };
            if let Some(end) = end {
                let at = rest[end].0;
                new_line = at == '\n';
                let skipped = usize::from(new_line || at == ' ');
                lines.push((rest[..end].to_vec(), wrapped));
                wrapped = !new_line;
                rest = &rest[end + skipped..];
                continue 'lines;
            }
        }
        break;
    }
    if !rest.is_empty() {
        lines.push((rest.to_vec(), wrapped));
    } else if new_line {
        lines.push((Vec::new(), false));
    }
    lines
}

/// How a text is made into lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Wrap {
    /// It is one.
    Single,
    /// As wide as this at most (`Font.split`).
    Split(i32),
    /// The same, for the chat: what goes on from a line before is set in a little
    /// (`ComponentRenderUtils.wrapComponents`).
    Chat(i32),
}

#[derive(Debug)]
struct Kept {
    /// What the lines are of: while it is held, nothing else is where it is.
    _of: Arc<Tag>,
    lines: Arc<[Arc<TextLine>]>,
    used: u64,
}

/// A player's hearts in the list (`PlayerTabOverlay.HealthState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListHealth {
    last: i32,
    displayed: i32,
    updated: i64,
    blink_until: i64,
}

impl ListHealth {
    fn new(value: i32) -> ListHealth {
        ListHealth {
            last: value,
            displayed: value,
            updated: 0,
            blink_until: 0,
        }
    }

    fn update(&mut self, value: i32, tick: i64) {
        if value != self.last {
            self.blink_until = tick + if value < self.last { 20 } else { 10 };
            self.last = value;
            self.updated = tick;
        }
        if tick - self.updated > 20 {
            self.displayed = value;
        }
    }

    fn blinks(&self, tick: i64) -> bool {
        self.blink_until > tick && (self.blink_until - tick) % 6 >= 3
    }
}

/// What the texts remember between frames: the lines they were made into, and the
/// player list's hearts.
#[derive(Debug, Default)]
pub struct HudMemory {
    lines: HashMap<(usize, Wrap), Kept>,
    frame: u64,
    healths: HashMap<u128, ListHealth>,
    /// The chat screen's edit box: where its text is shown from (a byte offset), and the
    /// text and caret it was last moved for.
    edit: (usize, String, usize),
}

impl HudMemory {
    /// Lets go of every line laid out (what they say changed: the key bindings).
    pub fn forget_lines(&mut self) {
        self.lines.clear();
    }

    /// A frame begins: what has not been asked for in a while is let go of.
    pub(super) fn begin(&mut self) {
        self.frame += 1;
        if self.frame.is_multiple_of(KEPT_FRAMES) {
            let frame = self.frame;
            self.lines.retain(|_, k| frame - k.used < KEPT_FRAMES);
        }
    }

    /// Where the edit box shows `value` from with the caret at `caret`
    /// (`EditBox.scrollTo`, each time the text or the caret has moved).
    pub(super) fn edit_display(
        &mut self,
        value: &str,
        caret: usize,
        font: &Font,
        width: i32,
    ) -> usize {
        let (display, text, at) = &mut self.edit;
        if text != value || *at != caret {
            text.clear();
            text.push_str(value);
            *at = caret;
            let mut d = (*display).min(value.len());
            while !value.is_char_boundary(d) {
                d -= 1;
            }
            let last = d + head_by_width(&value[d..], width, font).len();
            if caret == d {
                // (Back by what fits of the whole text's end, as vanilla counts it.)
                let back = tail_by_width(value, width, font).chars().count();
                for _ in 0..back {
                    d = value[..d].char_indices().next_back().map_or(0, |(i, _)| i);
                }
            }
            if caret > last {
                // On by as many characters as the caret is past the end.
                let past = value[last..caret].chars().count();
                for _ in 0..past {
                    d += value[d..].chars().next().map_or(0, char::len_utf8);
                }
            } else if caret <= d {
                d = caret;
            }
            *display = d.min(value.len());
        }
        self.edit.0
    }

    pub(super) fn lines(
        &mut self,
        text: &Arc<Tag>,
        wrap: Wrap,
        font: &Font,
        lang: &dyn Lang,
    ) -> Arc<[Arc<TextLine>]> {
        let frame = self.frame;
        let kept = self
            .lines
            .entry((Arc::as_ptr(text) as usize, wrap))
            .or_insert_with(|| {
                let chars = characters(text, lang);
                let line = |chars| Arc::new(TextLine::new(chars, font));
                let lines = match wrap {
                    Wrap::Single => vec![line(chars)],
                    Wrap::Split(width) => split(&chars, width, font)
                        .into_iter()
                        .map(|(chars, _)| line(chars))
                        .collect(),
                    Wrap::Chat(width) => {
                        let mut lines: Vec<_> = split(&chars, width, font)
                            .into_iter()
                            .map(|(mut chars, wrapped)| {
                                if wrapped {
                                    chars.insert(0, (' ', Style::default()));
                                }
                                line(chars)
                            })
                            .collect();
                        if lines.is_empty() {
                            lines.push(line(Vec::new()));
                        }
                        lines
                    }
                };
                Kept {
                    _of: text.clone(),
                    lines: lines.into(),
                    used: frame,
                }
            });
        kept.used = frame;
        kept.lines.clone()
    }
}

/// A line of text, in the game's own font: plain (`text`) or styled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudText {
    pub text: String,
    pub line: Option<Arc<TextLine>>,
    /// Where it begins, in GUI units, and how much larger than those it is.
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    /// `0xAARRGGBB`: what has a colour of its own takes the alpha alone.
    pub color: u32,
    pub shadow: bool,
    /// How many of the layout's `over` are under it.
    pub at: usize,
    /// It is in the `minecraft:alt` font (an enchanting table's words).
    pub alt: bool,
}

/// A player's face (`PlayerFaceExtractor`), eight by eight units.
#[derive(Debug, Clone, PartialEq)]
pub struct HudFace {
    pub skin: Arc<SkinSource>,
    pub x: i32,
    pub y: i32,
    pub hat: bool,
    pub flipped: bool,
    /// How many of the layout's `over` are under it.
    pub at: usize,
}

/// What the texts are laid out with.
pub(super) struct Words<'a, 'v> {
    pub sprites: &'a HudSprites,
    pub font: &'a Font,
    pub lang: &'a dyn Lang,
    pub view: &'a HudView<'v>,
    pub memory: &'a mut HudMemory,
}

impl Words<'_, '_> {
    fn line(&mut self, text: &Arc<Tag>) -> Arc<TextLine> {
        self.memory
            .lines(text, Wrap::Single, self.font, self.lang)
            .first()
            .cloned()
            .unwrap_or_default()
    }

    fn split(&mut self, text: &Arc<Tag>, width: i32) -> Arc<[Arc<TextLine>]> {
        self.memory
            .lines(text, Wrap::Split(width), self.font, self.lang)
    }
}

/// `ARGB.black(float)`.
fn black(alpha: f32) -> u32 {
    (mth::floor_f32(alpha * 255.0) as u32) << 24
}

/// How much of a chat message still shows, `age` ticks after it came in
/// (`AlphaCalculator.timeBased`).
fn chat_alpha(age: i32) -> f32 {
    let t = 1.0 - f64::from(age) / 200.0;
    let t = (t * 10.0).clamp(0.0, 1.0);
    (t * t) as f32
}

/// `Mth.positiveCeilDiv`.
fn ceil_div(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
}

impl HudLayout {
    /// The next line of text, empty.
    fn next_text(&mut self) -> &mut HudText {
        if self.texts_used == self.texts.len() {
            self.texts.push(HudText::default());
        }
        let at = self.over.quads().len();
        let t = &mut self.texts[self.texts_used];
        self.texts_used += 1;
        t.text.clear();
        t.line = None;
        t.scale = 1.0;
        t.at = at;
        t.alt = false;
        t
    }

    /// The text laid out last is in the `minecraft:alt` font.
    pub(super) fn mark_alt(&mut self) {
        if let Some(t) = self
            .texts_used
            .checked_sub(1)
            .and_then(|i| self.texts.get_mut(i))
        {
            t.alt = true;
        }
    }

    pub(super) fn text(
        &mut self,
        text: std::fmt::Arguments<'_>,
        x: i32,
        y: i32,
        color: u32,
        shadow: bool,
    ) {
        use std::fmt::Write;
        let t = self.next_text();
        let _ = t.text.write_fmt(text);
        (t.x, t.y, t.color, t.shadow) = (x as f32, y as f32, color, shadow);
    }

    /// A styled line, `scale` times as large about `origin` (`GuiGraphics.text` under a
    /// translated and scaled pose).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn styled(
        &mut self,
        line: &Arc<TextLine>,
        origin: [i32; 2],
        scale: i32,
        x: i32,
        y: i32,
        color: u32,
        shadow: bool,
    ) {
        // (`text` adds nothing that is not seen at all.)
        if color >> 24 == 0 || line.chars.is_empty() {
            return;
        }
        let t = self.next_text();
        t.line = Some(line.clone());
        t.scale = scale as f32;
        t.x = (origin[0] + x * scale) as f32;
        t.y = (origin[1] + y * scale) as f32;
        (t.color, t.shadow) = (color, shadow);
    }

    fn plain(&mut self, line: &Arc<TextLine>, x: i32, y: i32, color: u32, shadow: bool) {
        self.styled(line, [0, 0], 1, x, y, color, shadow);
    }

    /// `extractSelectedItemName`.
    pub(super) fn selected_name(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some((name, left)) = &v.hud.texts.selected else {
            return;
        };
        let line = w.line(name);
        let x = (v.width - line.width) / 2;
        let y = v.height - 59 + if v.hud.survival { 0 } else { 14 };
        let alpha = ((*left as f32 * 256.0 / 10.0) as i32).min(255);
        if alpha > 0 {
            self.plain(&line, x, y, (alpha as u32) << 24 | 0xFF_FFFF, true);
        }
    }

    /// `BossHealthOverlay.extractRenderState`.
    pub(super) fn bosses(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let s = w.sprites;
        let mut y = 12;
        for bar in v.hud.texts.bosses.iter() {
            let x = v.width / 2 - 91;
            let (color, notches) = (usize::from(bar.color), usize::from(bar.overlay));
            let progress = bar.progress(v.now);
            // `Mth.lerpDiscrete`.
            let full = mth::floor_f32(progress * 181.0) + i32::from(progress > 0.0);
            for (filled, width) in [(0, 182), (1, full)] {
                if width <= 0 {
                    continue;
                }
                let overlay = notches.checked_sub(1).and_then(|n| s.boss_notches.get(n));
                let sprites = [s.boss_bars.get(color), overlay];
                for sprite in sprites.into_iter().flatten() {
                    self.over.blit_part(
                        &sprite[filled],
                        s.texture,
                        [182, 5],
                        [0, 0],
                        x,
                        y,
                        width,
                        5,
                        WHITE,
                    );
                }
            }
            let name = w.line(&bar.name);
            self.plain(&name, v.width / 2 - name.width / 2, y - LINE, WHITE, true);
            y += 10 + LINE;
            if y >= v.height / 3 {
                break;
            }
        }
    }

    /// `displayScoreboardSidebar`.
    pub(super) fn sidebar(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some(bar) = &v.hud.texts.sidebar else {
            return;
        };
        let title = w.line(&bar.title);
        let spacer = text_width(w.font, ": ");
        let lines = bar.lines.iter();
        let lines: Vec<_> = lines.map(|(n, s)| (w.line(n), w.line(s))).collect();
        let width = lines.iter().fold(title.width, |width, (name, score)| {
            let score = if score.width > 0 {
                spacer + score.width
            } else {
                0
            };
            width.max(name.width + score)
        });
        let count = lines.len() as i32;
        let bottom = v.height / 2 + count * LINE / 3;
        let left = v.width - width - 3;
        let right = v.width - 3 + 2;
        let header = bottom - count * LINE;
        self.over
            .fill(left - 2, header - LINE - 1, right, header - 1, black(0.4));
        self.over
            .fill(left - 2, header - 1, right, bottom, black(0.3));
        let x = left + width / 2 - title.width / 2;
        self.plain(&title, x, header - LINE, WHITE, false);
        for (i, (name, score)) in (0..).zip(&lines) {
            let y = header + i * LINE;
            self.plain(name, left, y, WHITE, false);
            self.plain(score, right - score.width, y, WHITE, false);
        }
    }

    /// `extractOverlayMessage`.
    pub(super) fn overlay_message(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some(overlay) = &v.hud.texts.overlay else {
            return;
        };
        let t = overlay.time as f32 - v.partial;
        let alpha = ((t * 255.0 / 20.0) as i32).min(255);
        if alpha <= 0 {
            return;
        }
        let colour = if overlay.animated {
            hsv_to_rgb(t / 50.0, 0.7, 0.6)
        } else {
            0xFF_FFFF
        };
        let line = w.line(&overlay.text);
        let origin = [v.width / 2, v.height - 68];
        let color = (alpha as u32) << 24 | colour;
        self.styled(&line, origin, 1, -line.width / 2, -4, color, true);
    }

    /// `extractTitle`.
    pub(super) fn title(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some(title) = &v.hud.texts.title else {
            return;
        };
        let t = title.time as f32 - v.partial;
        let mut alpha = 255;
        if title.time > title.fade_out + title.stay {
            let gone = (title.fade_in + title.stay + title.fade_out) as f32 - t;
            alpha = (gone * 255.0 / title.fade_in as f32) as i32;
        }
        if title.time <= title.fade_out {
            alpha = (t * 255.0 / title.fade_out as f32) as i32;
        }
        let alpha = alpha.clamp(0, 255);
        if alpha == 0 {
            return;
        }
        let color = (alpha as u32) << 24 | 0xFF_FFFF;
        let origin = [v.width / 2, v.height / 2];
        let line = w.line(&title.title);
        self.styled(&line, origin, 4, -line.width / 2, -10, color, true);
        if let Some(subtitle) = &title.subtitle {
            let line = w.line(subtitle);
            self.styled(&line, origin, 2, -line.width / 2, 5, color, true);
        }
    }

    /// `ChatComponent.extractRenderState`, for a chat that is not typed in.
    pub(super) fn chat(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        // (`Hud.extractChat`: the chat screen draws it then.)
        if v.chat.is_some() {
            return;
        }
        let bottom = v.height - 40;
        // The lines that show, the newest first: the first of them is the lowest.
        let mut shown: Vec<(&ChatMessage, f32, Arc<TextLine>)> = Vec::new();
        'messages: for message in v.hud.texts.chat.iter() {
            let alpha = chat_alpha(v.hud.ticks.wrapping_sub(message.added));
            // (The ones before it are older.)
            if alpha <= 1e-5 {
                break;
            }
            let icon = message.tag.is_some_and(|t| t.icon);
            let width = CHAT_WIDTH - if icon { CHAT_ICON } else { 0 };
            let lines = w
                .memory
                .lines(&message.content, Wrap::Chat(width), w.font, w.lang);
            for line in lines.iter().rev() {
                if shown.len() == CHAT_LINES.min(CHAT_KEPT) {
                    break 'messages;
                }
                shown.push((message, alpha, line.clone()));
            }
        }
        // The highest first: the ground of all of them, and then what they say.
        for (i, (_, alpha, _)) in shown.iter().enumerate().rev() {
            let y = bottom - i as i32 * LINE;
            let ground = black(alpha * CHAT_GROUND);
            self.over.fill(0, y - LINE, CHAT_WIDTH + 12, y, ground);
        }
        for (i, (message, alpha, line)) in shown.iter().enumerate().rev() {
            let y = bottom - i as i32 * LINE;
            self.plain(line, 4, y - 8, white(*alpha), true);
            if let Some(tag) = message.tag {
                let colour = black(*alpha) | tag.colour;
                self.over.fill(0, y - LINE, 2, y, colour);
            }
        }
    }

    /// The player list, while it is asked for (`PlayerTabOverlay.extractRenderState`).
    pub(super) fn player_list(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some(list) = &v.hud.texts.players else {
            // (`setVisible`: the hearts begin anew when it shows again.)
            w.memory.healths.clear();
            return;
        };
        let names: Vec<_> = list.players.iter().map(|p| w.line(&p.name)).collect();
        let spacer = text_width(w.font, " ");
        let name_width = names.iter().map(|n| n.width).max().unwrap_or(0);
        let scores: Vec<Option<Arc<TextLine>>> = list
            .players
            .iter()
            .map(|p| match (list.scores, &p.score) {
                (ListScores::Numbers, Some((_, text))) => Some(w.line(text)),
                _ => None,
            })
            .collect();
        let score_width = match list.scores {
            ListScores::None => 0,
            ListScores::Hearts => 90,
            ListScores::Numbers => scores
                .iter()
                .flatten()
                .filter(|s| s.width > 0)
                .map(|s| spacer + s.width)
                .max()
                .unwrap_or(0),
        };
        let healths = &mut w.memory.healths;
        healths.retain(|id, _| list.players.iter().any(|p| p.id == *id));
        let slots = list.players.len() as i32;
        let (mut rows, mut columns) = (slots, 1);
        while rows > LIST_ROWS {
            columns += 1;
            rows = (slots + columns - 1) / columns;
        }
        let face = if list.faces { 9 } else { 0 };
        let slot_width =
            (columns * (face + name_width + score_width + 13)).min(v.width - 50) / columns;
        let middle = v.width / 2;
        let all = slot_width * columns + (columns - 1) * 5;
        let first = middle - all / 2;
        let header = list.header.as_ref().map(|t| w.split(t, v.width - 50));
        let footer = list.footer.as_ref().map(|t| w.split(t, v.width - 50));
        let widest = [&header, &footer]
            .into_iter()
            .flatten()
            .flat_map(|lines| lines.iter())
            .fold(all, |widest, line| widest.max(line.width));
        let mut y = 10;
        let sides = [middle - widest / 2 - 1, middle + widest / 2 + 1];
        let around = |layout: &mut HudLayout, lines: &[Arc<TextLine>], y: &mut i32| {
            let bottom = *y + lines.len() as i32 * LINE;
            layout
                .over
                .fill(sides[0], *y - 1, sides[1], bottom, LIST_GROUND);
            for line in lines {
                layout.plain(line, middle - line.width / 2, *y, WHITE, true);
                *y += LINE;
            }
        };
        if let Some(lines) = &header {
            around(self, lines, &mut y);
            y += 1;
        }
        self.over
            .fill(sides[0], y - 1, sides[1], y + rows * LINE, LIST_GROUND);
        for (i, p) in (0..).zip(&list.players) {
            let left = first + i / rows * (slot_width + 5);
            let top = y + i % rows * LINE;
            self.over
                .fill(left, top, left + slot_width, top + 8, LIST_SLOT);
            let mut x = left;
            if list.faces {
                self.faces.push(HudFace {
                    skin: p.skin.clone(),
                    x,
                    y: top,
                    hat: p.show_hat,
                    flipped: p.flipped,
                    at: self.over.quads().len(),
                });
                x += face;
            }
            let color = if p.spectator { LIST_SPECTATOR } else { WHITE };
            self.plain(&names[i as usize], x, top, color, true);
            let from = x + name_width + 1;
            let to = from + score_width;
            if !p.spectator && to - from > 5 {
                if list.scores == ListScores::Hearts {
                    self.list_hearts(w, p, top, from, to);
                } else if let Some(score) = &scores[i as usize] {
                    self.plain(score, to - score.width, top, WHITE, true);
                }
            }
            let ping = match p.latency {
                ..0 => 0,
                0..150 => 5,
                150..300 => 4,
                300..600 => 3,
                600..1000 => 2,
                _ => 1,
            };
            self.over.blit(
                &w.sprites.ping[ping],
                w.sprites.texture,
                left + slot_width - 11,
                top,
                10,
                8,
                WHITE,
            );
        }
        if let Some(lines) = &footer {
            y += rows * LINE + 1;
            around(self, lines, &mut y);
        }
    }

    /// `extractTablistHearts`.
    fn list_hearts(
        &mut self,
        w: &mut Words<'_, '_>,
        p: &ListedPlayer,
        y: i32,
        left: i32,
        right: i32,
    ) {
        let tick = i64::from(w.view.hud.ticks);
        let score = p.score.as_ref().map_or(0, |s| s.0);
        let health = w
            .memory
            .healths
            .entry(p.id)
            .or_insert_with(|| ListHealth::new(score));
        health.update(score, tick);
        let (displayed, blink) = (health.displayed, health.blinks(tick));
        let full = ceil_div(score.max(displayed), 2);
        let hearts = score.max(displayed.max(20)) / 2;
        if full <= 0 {
            return;
        }
        let per_heart = mth::floor_f32(((right - left - 4) as f32 / hearts as f32).min(9.0));
        if per_heart <= 3 {
            let part = (score as f32 / 20.0).clamp(0.0, 1.0);
            let colour =
                (((1.0 - part) * 255.0) as u32) << 16 | ((part * 255.0) as u32) << 8 | BLACK;
            let number = format!("{:?}", score as f32 / 2.0);
            let format = w.lang.get("multiplayer.player.list.hp").unwrap_or("%s hp");
            let long = format.replacen("%s", &number, 1);
            let text = if right - text_width(w.font, &long) >= left {
                long
            } else {
                number
            };
            let x = (right + left - text_width(w.font, &text)) / 2;
            self.text(format_args!("{text}"), x, y, colour, true);
            return;
        }
        let (s, t) = (w.sprites, w.sprites.texture);
        let mut heart = |i: i32, kind: Heart, half: bool, blinking: bool| {
            let sprite = s.heart(kind, false, half, blinking);
            self.over
                .blit(sprite, t, left + i * per_heart, y, 9, 9, WHITE);
        };
        for i in full..hearts {
            heart(i, Heart::Container, false, blink);
        }
        for i in 0..full {
            heart(i, Heart::Container, false, blink);
            let own = i * 2 + 1;
            if blink && own < displayed {
                heart(i, Heart::Normal, false, true);
            }
            if blink && own == displayed {
                heart(i, Heart::Normal, true, true);
            }
            let (kind, blinking) = if i >= 10 {
                (Heart::Absorbing, true)
            } else {
                (Heart::Normal, false)
            };
            if own < score {
                heart(i, kind, false, blinking);
            }
            if own == score {
                heart(i, kind, true, blinking);
            }
        }
    }

    pub(super) fn words(&mut self, w: &mut Words<'_, '_>) {
        self.sidebar(w);
        self.overlay_message(w);
        self.title(w);
        self.chat(w);
        self.player_list(w);
        self.chat_screen(w);
    }

    /// `ChatScreen.extractRenderState`, over the rest of the HUD: the edit box's ground,
    /// the chat in the foreground (`DisplayMode.FOREGROUND`: its focused height, every
    /// line at full strength, scrolled, with the scroll bar), and the edit box.
    fn chat_screen(&mut self, w: &mut Words<'_, '_>) {
        let v = w.view;
        let Some(input) = v.chat else { return };
        let (width, height) = (v.width, v.height);
        self.over
            .fill(2, height - 14, width - 2, height - 2, CHAT_INPUT_GROUND);
        // Every line kept, the newest first (`trimmedMessages`).
        let mut lines: Vec<(&ChatMessage, Arc<TextLine>)> = Vec::new();
        let mut newer = 0;
        let mut found = input.newest.is_none();
        'messages: for message in v.hud.texts.chat.iter() {
            if input
                .newest
                .as_ref()
                .is_some_and(|n| Arc::ptr_eq(n, message))
            {
                found = true;
            }
            let icon = message.tag.is_some_and(|t| t.icon);
            let wrap = CHAT_WIDTH - if icon { CHAT_ICON } else { 0 };
            let split = w
                .memory
                .lines(&message.content, Wrap::Chat(wrap), w.font, w.lang);
            for line in split.iter().rev() {
                if lines.len() == CHAT_KEPT {
                    break 'messages;
                }
                if !found {
                    newer += 1;
                }
                lines.push((message, line.clone()));
            }
        }
        // `addMessageToDisplayQueue` while scrolled up: `scrollChat(1)` a new line.
        let total = lines.len() as i32;
        let (mut scroll, mut new_since) = (input.scroll, input.new_since_scroll);
        if scroll > 0 && found && newer > 0 {
            scroll += newer;
            new_since = true;
        }
        // (The screen takes the new lines in from `chat_shown` for the frames after.)
        // `scrollChat`'s bounds.
        scroll = scroll.min(total - CHAT_FOCUSED_LINES);
        if scroll <= 0 {
            scroll = 0;
            new_since = false;
        }
        self.chat_shown = Some(ChatShown {
            lines: lines.len(),
            added: if found { newer } else { 0 },
            newest: v.hud.texts.chat.first().cloned(),
        });
        let bottom = height - 40;
        let page = &lines[scroll as usize
            ..lines
                .len()
                .min(scroll as usize + CHAT_FOCUSED_LINES as usize)];
        for i in (0..page.len()).rev() {
            let y = bottom - i as i32 * LINE;
            self.over
                .fill(0, y - LINE, CHAT_WIDTH + 12, y, black(CHAT_GROUND));
        }
        for (i, (message, line)) in page.iter().enumerate().rev() {
            let y = bottom - i as i32 * LINE;
            self.plain(line, 4, y - 8, white(1.0), true);
            if let Some(tag) = message.tag {
                self.over.fill(0, y - LINE, 2, y, BLACK | tag.colour);
            }
        }
        // The scroll bar (its `y` is mostly negative, so it is mostly faint).
        let count = page.len() as i32;
        if total > 0 && count != total {
            let chat_height = count * LINE;
            let virtual_height = total * LINE;
            let y = scroll * chat_height / total - bottom;
            let bar = chat_height * chat_height / virtual_height;
            let alpha: u32 = if y > 0 { 170 } else { 96 };
            let colour = if new_since { 0xCC_3333 } else { 0x33_33AA };
            let x = 4 + CHAT_WIDTH + 4;
            self.over.fill(x, -y, x + 2, -y - bar, alpha << 24 | colour);
            self.over
                .fill(x + 2, -y, x + 1, -y - bar, alpha << 24 | 0xCC_CCCC);
        }
        self.edit_box(w, input);
    }

    /// The chat screen's `EditBox` (`extractWidgetRenderState`): no border, at 4 and
    /// 12 above the bottom, as wide as the screen less 4, the text from where it is
    /// scrolled to, the caret (a bar inside the text, `_` at its end) and the selection.
    fn edit_box(&mut self, w: &mut Words<'_, '_>, input: &ChatInput) {
        let v = w.view;
        let font = w.font;
        let (x, text_y, box_width) = (4, v.height - 12, v.width - 4);
        let value = input.text.as_str();
        let display = w.memory.edit_display(value, input.caret, font, box_width);
        let shown = head_by_width(&value[display..], box_width, font);
        let shown_end = display + shown.len();
        let on_screen = (display..=shown_end).contains(&input.caret);
        let mut draw_x = x;
        if !shown.is_empty() {
            let half = if on_screen {
                &value[display..input.caret]
            } else {
                shown
            };
            if !half.is_empty() {
                self.edit_text(half, input.draft, draw_x, text_y, font);
            }
            draw_x += text_width(font, half) + 1;
        }
        let insert = input.caret < value.len() || value.encode_utf16().count() >= CHAT_INPUT_MAX;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if input.caret > display {
                x + box_width
            } else {
                x
            };
        } else if insert {
            cursor_x -= 1;
            draw_x -= 1;
        }
        if !shown.is_empty() && on_screen && input.caret < shown_end {
            let rest = &value[input.caret..shown_end];
            self.edit_text(rest, input.draft, draw_x, text_y, font);
        }
        let anchor = input.anchor.clamp(display, shown_end);
        if anchor != input.caret {
            let highlight_x = x + text_width(font, &value[display..anchor]);
            let right = x + box_width;
            let (x0, x1) = (cursor_x.min(right), (highlight_x - 1).min(right));
            self.over.fill(
                x0.min(x1),
                text_y - 1,
                x0.max(x1),
                text_y + 1 + LINE,
                CHAT_INPUT_SELECTION,
            );
        }
        if input.caret_on && on_screen {
            // (The caret is not a draft's: it keeps the box's colour, upright.)
            if insert {
                self.over.fill(
                    cursor_x,
                    text_y - 1,
                    cursor_x + 1,
                    text_y + LINE,
                    CHAT_INPUT_TEXT,
                );
            } else {
                self.text(format_args!("_"), cursor_x, text_y, CHAT_INPUT_TEXT, true);
            }
        }
    }

    /// Part of the edit box's text: a draft's grey and slanted (`formatChat`'s style over
    /// the box's colour), else as it is.
    fn edit_text(&mut self, s: &str, draft: bool, x: i32, y: i32, font: &Font) {
        if draft {
            let style = Style {
                color: Some(CHAT_DRAFT),
                italic: true,
                ..Style::default()
            };
            let line = TextLine::new(s.chars().map(|c| (c, style)).collect(), font);
            self.plain(&Arc::new(line), x, y, CHAT_INPUT_TEXT, true);
        } else {
            self.text(format_args!("{s}"), x, y, CHAT_INPUT_TEXT, true);
        }
    }
}
#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use ruthenium_assets::font::Glyph;
    use ruthenium_assets::meta::GuiScaling;
    use ruthenium_protocol::nbt::Compound;
    use ruthenium_sim::hud::HudRecord;
    use ruthenium_sim::hud_text::{BossBar, HudTexts, Overlay, PlayerList, Title};
    use ruthenium_sim::scoreboard::Sidebar;

    use super::super::{HudHealth, HudOptions};
    use super::*;
    use crate::draw::SpriteInfo;

    /// Every character six wide, a space four.
    fn font() -> Font {
        let glyph = |advance| Glyph {
            advance,
            ..Glyph::MISSING
        };
        Font::of_glyphs((' '..='~').map(|c| (c, glyph(if c == ' ' { 4.0 } else { 6.0 }))))
    }

    fn text(s: &str) -> Arc<Tag> {
        Arc::new(Tag::String(s.to_owned()))
    }

    fn shown(line: &[(char, Style)]) -> String {
        line.iter().map(|(c, _)| *c).collect()
    }

    fn plain(s: &str) -> Vec<(char, Style)> {
        s.chars().map(|c| (c, Style::default())).collect()
    }

    /// What the game's own splitter makes of the same made-up texts.
    #[test]
    fn lines_break_where_the_games_do() {
        const WANT: [u64; 12] = [
            0x344F_5E97_C6EA_7B91,
            0xB716_7F2F_27BD_D911,
            0x6C6D_0102_800D_450B,
            0x3386_5A81_9BD0_BF61,
            0x58C2_FBD5_A9D9_621B,
            0xA95D_EAC9_947B_2D46,
            0x393F_F4AD_8F43_E0E4,
            0x2D7B_29C2_FD98_215F,
            0x58D0_94CB_BFCB_ADA0,
            0x748F_765D_A9E8_D3EE,
            0x9E5C_530C_DCC6_46BF,
            0x7C14_D678_D7F8_98AF,
        ];
        const ALPHABET: &str = "abcdefghij.";
        let mut glyphs: Vec<(char, Glyph)> = (0..)
            .zip(ALPHABET.chars())
            .map(|(k, c)| {
                let glyph = Glyph {
                    advance: if k == 9 { 0.0 } else { (2 + k % 7) as f32 },
                    bold_offset: if k % 4 == 3 { 0.5 } else { 1.0 },
                    ..Glyph::MISSING
                };
                (c, glyph)
            })
            .collect();
        glyphs.push((
            ' ',
            Glyph {
                advance: 4.0,
                ..Glyph::MISSING
            },
        ));
        let font = Font::of_glyphs(glyphs);
        let mut state = 20_261_125u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % n
        };
        for (batch, want) in WANT.into_iter().enumerate() {
            let mut hash = 0xCBF2_9CE4_8422_2325u64;
            let mut word = |v: i32| {
                for byte in v.to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01B3);
                }
            };
            for _ in 0..50 {
                let max = 1 + next(60) as i32;
                let runs = 1 + next(3);
                let mut parts = Vec::new();
                for _ in 0..runs {
                    let bold = next(3) == 0;
                    let colour = next(4);
                    let mut s = String::new();
                    for _ in 0..next(24) {
                        match next(16) as usize {
                            c @ 0..11 => s.push_str(&ALPHABET[c..=c]),
                            11..14 => s.push(' '),
                            14 => s.push(if next(3) == 0 { '\n' } else { 'b' }),
                            _ if next(2) == 0 => {
                                s.push('§');
                                let code = next(5) as usize;
                                s.push_str(&"clonm"[code..=code]);
                            }
                            _ => {}
                        }
                    }
                    let mut part = Compound::new();
                    part.put("text", Tag::String(s));
                    if bold {
                        part.put("bold", Tag::Byte(1));
                    }
                    if colour == 1 {
                        part.put("color", Tag::String("gold".into()));
                    }
                    parts.push(Tag::Compound(part));
                }
                let mut all = Compound::new();
                all.put("text", Tag::String(String::new()));
                all.put("extra", Tag::List(parts));
                let chars = characters(&Tag::Compound(all), &());
                for (line, wrapped) in split(&chars, max, &font) {
                    word(if wrapped { -2 } else { -1 });
                    for (c, s) in line {
                        word(c as i32);
                        let flags = [s.bold, s.italic, s.underlined, s.strikethrough];
                        word((0..).zip(flags).map(|(i, f)| i32::from(f) << i).sum());
                        word(s.color.map_or(-1, |c| c as i32));
                    }
                }
                word(-3);
            }
            assert_eq!(hash, want, "batch {batch}");
        }
    }

    #[test]
    fn lines_are_made_once_and_set_in_where_the_chat_goes_on() {
        let (f, mut m) = (font(), HudMemory::default());
        let t = text("aaa bbb\nc");
        let chat = m.lines(&t, Wrap::Chat(20), &f, &());
        let lines: Vec<String> = chat.iter().map(|l| shown(&l.chars)).collect();
        assert_eq!(lines, ["aaa", " bbb", "c"]);
        assert_eq!([chat[0].width, chat[1].width], [18, 22]);
        let split = m.lines(&t, Wrap::Split(20), &f, &());
        assert_eq!(shown(&split[1].chars), "bbb");
        assert!(Arc::ptr_eq(&chat, &m.lines(&t, Wrap::Chat(20), &f, &())));
        // An empty message is one empty line, an empty header none.
        let empty = text("");
        assert_eq!(m.lines(&empty, Wrap::Chat(20), &f, &()).len(), 1);
        assert_eq!(m.lines(&empty, Wrap::Split(20), &f, &()).len(), 0);
        // Ends of lines: a break at the end begins another, a space does not.
        let ends = |s: &str| super::split(&plain(s), 18, &f).len();
        assert_eq!([ends("a\n"), ends("aaa "), ends("a\n\nb")], [2, 1, 3]);
        // What has not been asked for in a while goes.
        for _ in 0..KEPT_FRAMES - 1 {
            m.begin();
        }
        assert_eq!(m.lines.len(), 4);
        m.lines(&t, Wrap::Chat(20), &f, &());
        m.begin();
        assert_eq!(m.lines.len(), 1);
    }

    struct Laid {
        layout: HudLayout,
        names: Vec<String>,
    }

    impl Laid {
        /// Each text: what it says, where, its scale and its colour.
        fn texts(&self) -> Vec<(String, [i32; 2], i32, u32)> {
            self.layout
                .texts()
                .iter()
                .map(|t| {
                    let says = match &t.line {
                        Some(line) => shown(&line.chars),
                        None => t.text.clone(),
                    };
                    (says, [t.x as i32, t.y as i32], t.scale as i32, t.color)
                })
                .collect()
        }

        /// Each quad over the items: its sprite (nothing for a fill), its place, colour.
        fn over(&self) -> Vec<(String, [i32; 4], u32)> {
            self.layout
                .over
                .quads()
                .iter()
                .map(|q| {
                    let name = if q.texture == crate::NO_TEXTURE {
                        String::new()
                    } else {
                        self.names[q.uv[0].floor() as usize].clone()
                    };
                    let [x0, y0, x1, y1] = q.rect.map(|v| v as i32);
                    (name, [x0, y0, x1 - x0, y1 - y0], q.color)
                })
                .collect()
        }
    }

    /// A HUD of 427 by 240 with nothing on it but its texts.
    fn lay_with(hud: &HudRecord, partial: f32, now: Instant, memory: &mut HudMemory) -> Laid {
        let names = RefCell::new(Vec::new());
        let sprites = HudSprites::with(7, |id| {
            let mut names = names.borrow_mut();
            names.push(id.path().to_owned());
            let n = (names.len() - 1) as f32;
            SpriteInfo {
                uv: [n, 0.0, n + 1.0, 1.0],
                width: 16,
                height: 16,
                scaling: GuiScaling::Stretch,
            }
        });
        let mut layout = HudLayout::new();
        let view = HudView {
            hud,
            partial,
            width: 427,
            height: 240,
            millis: 0,
            now,
            options: HudOptions::default(),
            chat: None,
        };
        let mut health = HudHealth::default();
        layout.build(&sprites, &font(), &(), &view, &mut health, memory);
        Laid {
            layout,
            names: names.into_inner(),
        }
    }

    /// The HUD of `lay_with` with the chat screen open over it.
    fn lay_typing(hud: &HudRecord, input: &ChatInput, memory: &mut HudMemory) -> Laid {
        let sprites = HudSprites::with(7, |_| SpriteInfo {
            uv: [0.0, 0.0, 1.0, 1.0],
            width: 16,
            height: 16,
            scaling: GuiScaling::Stretch,
        });
        let mut layout = HudLayout::new();
        let view = HudView {
            hud,
            partial: 0.0,
            width: 427,
            height: 240,
            millis: 0,
            now: Instant::now(),
            options: HudOptions::default(),
            chat: Some(input),
        };
        layout.build(
            &sprites,
            &font(),
            &(),
            &view,
            &mut HudHealth::default(),
            memory,
        );
        Laid {
            layout,
            names: vec![String::new()],
        }
    }

    /// `n` messages `m0` to `m{n-1}`, long faded, the newest first.
    fn many(n: usize) -> HudRecord {
        use ruthenium_sim::hud_text::HudTextState;
        let mut state = HudTextState::default();
        for i in 0..n {
            state.system_message(0, Tag::String(format!("m{i}")));
        }
        let mut texts = HudTexts::default();
        state.fill(
            &mut texts,
            0,
            "",
            false,
            false,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        HudRecord {
            ticks: 1000,
            texts,
            ..HudRecord::default()
        }
    }

    #[test]
    fn the_chat_screen_shows_twenty_lines_at_full_strength_scrolled() {
        let hud = many(25);
        // Not typed in, faded long ago: nothing.
        assert!(
            lay_with(&hud, 0.0, Instant::now(), &mut HudMemory::default())
                .texts()
                .is_empty()
        );
        let input = ChatInput::default();
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        let lines: Vec<_> = l
            .texts()
            .into_iter()
            .filter(|t| t.0.starts_with('m'))
            .collect();
        assert_eq!(lines.len(), 20);
        // The highest first; the newest lowest, at the chat's bottom less 8.
        assert_eq!(lines[0], ("m5".into(), [4, 21], 1, WHITE));
        assert_eq!(lines[19], ("m24".into(), [4, 192], 1, WHITE));
        assert_eq!(l.layout.chat_shown.as_ref().unwrap().lines, 25);
        // Each line's ground, the edit box's ground, and the scroll bar's two parts.
        let over = l.over();
        assert!(over.contains(&(String::new(), [2, 226, 423, 12], 0x8000_0000)));
        assert!(over.contains(&(String::new(), [0, 191, 332, 9], 0x7F00_0000)));
        // (Its `y` is the scroll's share less the chat's bottom: from 200 up, 180 * 180 / 225
        // long.)
        assert!(over.contains(&(String::new(), [328, 56, 2, 144], 96 << 24 | 0x33_33AA)));
        assert!(over.contains(&(String::new(), [329, 56, 1, 144], 96 << 24 | 0xCC_CCCC)));

        // Scrolled up three, and too far.
        let up = |scroll| {
            let input = ChatInput {
                scroll,
                ..ChatInput::default()
            };
            let l = lay_typing(&hud, &input, &mut HudMemory::default());
            l.texts()
                .into_iter()
                .rfind(|t| t.0.starts_with('m'))
                .unwrap()
                .0
        };
        assert_eq!(up(3), "m21");
        assert_eq!(up(100), "m19");
        assert_eq!(up(-4), "m24");
        // Two lines since the newest it was scrolled at: up two more, and the bar red.
        let input = ChatInput {
            scroll: 1,
            newest: Some(hud.texts.chat[2].clone()),
            ..ChatInput::default()
        };
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        let last = l
            .texts()
            .into_iter()
            .rfind(|t| t.0.starts_with('m'))
            .unwrap();
        assert_eq!(last.0, "m21");
        assert_eq!(l.layout.chat_shown.as_ref().unwrap().added, 2);
        assert!(l.over().iter().any(|q| q.2 == 96 << 24 | 0xCC_3333));
        // At the bottom it stays there.
        let input = ChatInput {
            newest: Some(hud.texts.chat[2].clone()),
            ..ChatInput::default()
        };
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        let last = l
            .texts()
            .into_iter()
            .rfind(|t| t.0.starts_with('m'))
            .unwrap();
        assert_eq!(last.0, "m24");
    }

    #[test]
    fn a_draft_shows_grey_and_slanted_but_its_caret_does_not() {
        let hud = HudRecord::default();
        let input = ChatInput {
            text: "hello".into(),
            caret: 2,
            anchor: 2,
            caret_on: true,
            draft: true,
            ..ChatInput::default()
        };
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        // In the same places as plain text, each half grey and slanted over the box's
        // colour (the style's colour wins, with the box's alpha).
        let texts = l.layout.texts();
        assert_eq!(
            l.texts(),
            [
                ("he".into(), [4, 228], 1, CHAT_INPUT_TEXT),
                ("llo".into(), [16, 228], 1, CHAT_INPUT_TEXT)
            ]
        );
        for t in texts {
            let line = t.line.as_ref().unwrap();
            assert!(t.shadow);
            assert!(
                line.chars
                    .iter()
                    .all(|(_, s)| s.italic && s.color == Some(0xAA_AAAA) && !s.bold)
            );
        }
        // The bar between stays the box's colour.
        assert!(
            l.over()
                .contains(&(String::new(), [16, 227, 1, 10], CHAT_INPUT_TEXT))
        );
        // At the end, the `_` is upright and plain.
        let input = ChatInput {
            caret: 5,
            anchor: 5,
            ..input
        };
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        let caret = &l.layout.texts()[1];
        assert_eq!((caret.text.as_str(), caret.line.is_none()), ("_", true));
        // Not a draft: plain text, as before.
        let input = ChatInput {
            draft: false,
            ..input
        };
        let l = lay_typing(&hud, &input, &mut HudMemory::default());
        assert!(l.layout.texts().iter().all(|t| t.line.is_none()));
    }

    #[test]
    fn the_edit_box_shows_the_text_the_caret_and_the_selection() {
        let hud = HudRecord::default();
        let typed = |text: &str, caret, anchor, caret_on, memory: &mut HudMemory| {
            let input = ChatInput {
                text: text.into(),
                caret,
                anchor,
                caret_on,
                ..ChatInput::default()
            };
            let l = lay_typing(&hud, &input, memory);
            (l.texts(), l.over())
        };
        const T: u32 = 0xFFE0_E0E0;
        let m = &mut HudMemory::default();
        // At the end: the text and `_` after it, when the caret blinks on.
        let (texts, _) = typed("hello", 5, 5, true, m);
        assert_eq!(
            texts,
            [
                ("hello".into(), [4, 228], 1, T),
                ("_".into(), [35, 228], 1, T)
            ]
        );
        let (texts, _) = typed("hello", 5, 5, false, m);
        assert_eq!(texts, [("hello".into(), [4, 228], 1, T)]);
        // Inside: the text in two, a bar between, and the selection back to the start.
        let (texts, over) = typed("hello", 2, 0, true, m);
        assert_eq!(
            texts,
            [
                ("he".into(), [4, 228], 1, T),
                ("llo".into(), [16, 228], 1, T)
            ]
        );
        assert!(over.contains(&(String::new(), [16, 227, 1, 10], T)));
        assert!(over.contains(&(String::new(), [3, 227, 13, 11], 0x8000_00FF)));
        // Longer than the box: shown from where the caret's end fits.
        let long = "a".repeat(100);
        let (texts, _) = typed(&long, 100, 100, true, m);
        assert_eq!(texts[0], ("a".repeat(70), [4, 228], 1, T));
        assert_eq!(texts[1], ("_".into(), [425, 228], 1, T));
        // Home: back to its start.
        let (texts, _) = typed(&long, 0, 0, false, m);
        assert_eq!(texts[0], ("a".repeat(70), [4, 228], 1, T));
        let (texts, _) = typed(&long, 99, 99, false, m);
        assert_eq!(texts, [("a".repeat(70), [4, 228], 1, T)]);
        let (texts, _) = typed(&long, 100, 100, false, m);
        assert_eq!(texts, [("a".repeat(70), [4, 228], 1, T)]);
    }

    fn lay(texts: HudTexts) -> Laid {
        lay_at(texts, 0.0)
    }

    fn lay_at(texts: HudTexts, partial: f32) -> Laid {
        let hud = HudRecord {
            texts,
            ..HudRecord::default()
        };
        lay_with(&hud, partial, Instant::now(), &mut HudMemory::default())
    }

    const CLEAR: u32 = 0xFF_FFFF;

    #[test]
    fn the_selected_items_name_stands_over_the_bars_and_fades() {
        let named = |left, survival, spectator| {
            let hud = HudRecord {
                survival,
                spectator,
                health: 20.0,
                texts: HudTexts {
                    selected: Some((text("Sword"), left)),
                    ..HudTexts::default()
                },
                ..HudRecord::default()
            };
            let l = lay_with(&hud, 0.5, Instant::now(), &mut HudMemory::default());
            l.texts()
        };
        // 30 wide, in the middle of 427.
        assert_eq!(
            named(40, true, false),
            [("Sword".into(), [198, 181], 1, WHITE)]
        );
        assert_eq!(named(40, false, false)[0].1, [198, 195]);
        // In its last ten ticks it fades, in whole ticks.
        assert_eq!(named(10, true, false)[0].3, WHITE);
        assert_eq!(named(5, true, false)[0].3, 128 << 24 | CLEAR);
        assert_eq!(named(1, true, false)[0].3, 25 << 24 | CLEAR);
        assert!(named(40, false, true).is_empty());
    }

    #[test]
    fn the_message_over_the_hotbar_fades_in_its_last_second() {
        let at = |time, partial, animated| {
            lay_at(
                HudTexts {
                    overlay: Some(Overlay {
                        text: text("abc"),
                        time,
                        animated,
                    }),
                    ..HudTexts::default()
                },
                partial,
            )
            .texts()
        };
        // 18 wide, and its middle 68 over the bottom.
        assert_eq!(
            at(60, 0.0, false),
            [("abc".into(), [213 - 9, 168], 1, WHITE)]
        );
        assert_eq!(at(10, 0.5, false)[0].3, 121 << 24 | CLEAR);
        assert!(at(1, 1.0, false).is_empty());
        // The rainbow: a fifth of the way round is between yellow and green.
        assert_eq!(
            at(10, 0.0, true)[0].3,
            127 << 24 | hsv_to_rgb(0.2, 0.7, 0.6)
        );
        assert_eq!(hsv_to_rgb(0.2, 0.7, 0.6), 0x83_992D);
    }

    fn title(time: i32, partial: f32) -> Vec<(String, [i32; 2], i32, u32)> {
        lay_at(
            HudTexts {
                title: Some(Title {
                    title: text("Hello"),
                    subtitle: Some(text("abc")),
                    time,
                    fade_in: 10,
                    stay: 70,
                    fade_out: 20,
                }),
                ..HudTexts::default()
            },
            partial,
        )
        .texts()
    }

    #[test]
    fn titles_are_large_and_fade_in_and_out() {
        // Four and two times as large, about the middle: (213, 120).
        assert_eq!(
            title(50, 0.0),
            [
                ("Hello".into(), [213 - 60, 120 - 40], 4, WHITE),
                ("abc".into(), [213 - 18, 120 + 10], 2, WHITE),
            ]
        );
        let alpha = |time, partial| title(time, partial).first().map(|t| t.3 >> 24);
        assert_eq!(alpha(100, 0.0), None);
        assert_eq!(alpha(100, 0.5), Some(12));
        assert_eq!(alpha(95, 0.0), Some(127));
        assert_eq!(alpha(91, 0.5), Some(242));
        assert_eq!(alpha(90, 0.0), Some(255));
        assert_eq!([alpha(21, 0.9), alpha(20, 0.0)], [Some(255); 2]);
        assert_eq!(alpha(10, 0.5), Some(121));
        assert_eq!(alpha(1, 1.0), None);
        // Without a subtitle there is the title alone, and nothing without one that shows.
        assert!(lay(HudTexts::default()).texts().is_empty());
    }

    fn chat_of(messages: &[(&str, i32)], ticks: i32) -> Laid {
        use ruthenium_sim::hud_text::HudTextState;
        let mut state = HudTextState::default();
        for (says, added) in messages {
            state.system_message(*added, Tag::String((*says).to_owned()));
        }
        let mut texts = HudTexts::default();
        state.fill(
            &mut texts,
            0,
            "",
            false,
            false,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        let hud = HudRecord {
            ticks,
            texts,
            ..HudRecord::default()
        };
        lay_with(&hud, 0.0, Instant::now(), &mut HudMemory::default())
    }

    #[test]
    fn the_chat_stands_over_its_ground_and_fades_after_nine_seconds() {
        let l = chat_of(&[("first", 0), ("second", 10)], 10);
        assert_eq!(
            l.texts(),
            [
                ("first".into(), [4, 183], 1, WHITE),
                ("second".into(), [4, 192], 1, WHITE)
            ]
        );
        assert!(l.layout.texts().iter().all(|t| t.shadow));
        let half = 127 << 24;
        let grey = 0xFFD0_D0D0;
        assert_eq!(
            l.over(),
            [
                (String::new(), [0, 182, 332, 9], half),
                (String::new(), [0, 191, 332, 9], half),
                (String::new(), [0, 182, 2, 9], grey),
                (String::new(), [0, 191, 2, 9], grey),
            ]
        );
        // All the ground is under all the words.
        assert!(l.layout.texts().iter().all(|t| t.at >= 2));
        // At 190 ticks it is half way out: a quarter of it is left.
        let l = chat_of(&[("old", 0)], 190);
        assert_eq!(l.texts()[0].3, 63 << 24 | CLEAR);
        let colours: Vec<u32> = l.over().iter().map(|q| q.2).collect();
        assert_eq!(colours, [31 << 24, 63 << 24 | 0xD0_D0D0]);
        assert_eq!(chat_of(&[("old", 0)], 180).texts()[0].3, WHITE);
        assert!(chat_of(&[("old", 0)], 200).texts().is_empty());
        assert!(chat_of(&[("old", 0)], 200).over().is_empty());
        // What is left of it at the very end is not seen.
        assert!(chat_of(&[("old", 0)], 199).texts().is_empty());
        // The newer ones show below it all the same.
        assert_eq!(chat_of(&[("old", 0), ("new", 150)], 250).texts().len(), 1);
    }

    #[test]
    fn the_chat_shows_ten_lines_the_newest_lowest() {
        let long = "x".repeat(60);
        let many: Vec<(&str, i32)> = (0..4).flat_map(|_| [("top", 0), (&*long, 0)]).collect();
        // Four times a line and two: the two lowest of the last, and so on.
        let l = chat_of(&many, 0);
        let texts = l.texts();
        assert_eq!(texts.len(), 10);
        assert_eq!(texts[9].0, format!(" {}", "x".repeat(60 - 53)));
        assert_eq!(texts[8].0, "x".repeat(53));
        assert_eq!((texts[7].0.as_str(), texts[0].0.len()), ("top", 8));
        assert_eq!((texts[0].1, texts[9].1), ([4, 111], [4, 192]));
    }

    #[test]
    fn a_changed_message_leaves_room_for_its_icon() {
        use ruthenium_protocol::packets::play::{
            BoundChatType, ChatDecoration, ChatParameter, ChatTypeRef, FilterMask, PlayerChat,
        };
        use ruthenium_sim::hud_text::{ChatTypes, HudTextState};
        let lines = |end: &str| {
            let kind = ChatDecoration {
                translation_key: "%s".into(),
                parameters: vec![ChatParameter::Content],
                style: None,
            };
            let says = "x".repeat(52);
            let chat = PlayerChat {
                global_index: 0,
                sender: 1,
                signature: Some(Box::new([1; 256])),
                content: "y".into(),
                time: 0,
                last_seen: Vec::new(),
                unsigned_content: Some(Tag::String(says + end)),
                filter: FilterMask::PassThrough,
                kind: BoundChatType {
                    kind: ChatTypeRef::Direct(Box::new(kind)),
                    name: Tag::String(String::new()),
                    target: None,
                },
            };
            let mut state = HudTextState::default();
            state
                .player_chat(0, 0, &chat, true, &ChatTypes::default())
                .unwrap();
            let mut hud = HudRecord::default();
            state.fill(
                &mut hud.texts,
                0,
                "",
                false,
                false,
                &Default::default(),
                &Default::default(),
                &Default::default(),
            );
            let l = lay_with(&hud, 0.0, Instant::now(), &mut HudMemory::default());
            (l.texts().len(), l.over()[l.over().len() - 1].2)
        };
        // What its sender wrote is in it: it fits, and nothing is beside it.
        assert_eq!(lines("y"), (1, 127 << 24));
        // It is not: less fits, and a dark bar is beside it.
        assert_eq!(lines(""), (2, 0xFF60_6060));
    }

    #[test]
    fn boss_bars_stand_under_their_names_from_the_top() {
        let bar = |id, name: &str, progress, color, overlay| {
            BossBar::new(id, text(name), progress, color, overlay)
        };
        let bosses = vec![bar(1, "abc", 0.5, 2, 0), bar(2, "", 0.0, 6, 4)];
        let l = lay(HudTexts {
            bosses: Arc::new(bosses),
            ..HudTexts::default()
        });
        assert_eq!(
            l.over(),
            [
                ("boss_bar/red_background".into(), [122, 12, 182, 5], WHITE),
                ("boss_bar/red_progress".into(), [122, 12, 91, 5], WHITE),
                ("boss_bar/white_background".into(), [122, 31, 182, 5], WHITE),
                (
                    "boss_bar/notched_20_background".into(),
                    [122, 31, 182, 5],
                    WHITE
                ),
            ]
        );
        assert_eq!(l.texts(), [("abc".into(), [213 - 9, 3], 1, WHITE)]);
        // The least that is left shows, and all of it all.
        let full = |progress| {
            let l = lay(HudTexts {
                bosses: Arc::new(vec![bar(1, "", progress, 0, 1)]),
                ..HudTexts::default()
            });
            let widths: Vec<i32> = l.over().iter().map(|q| q.1[2]).collect();
            widths
        };
        assert_eq!(full(0.0), [182, 182]);
        assert_eq!(full(0.001), [182, 182, 1, 1]);
        assert_eq!(full(1.0), [182, 182, 182, 182]);
        // A third of the height is the most they take: 80 of 240.
        let many = (0..9).map(|i| bar(i, "n", 0.0, 0, 0)).collect();
        let l = lay(HudTexts {
            bosses: Arc::new(many),
            ..HudTexts::default()
        });
        assert_eq!(l.texts().len(), 4);
    }

    #[test]
    fn the_sidebar_is_as_wide_as_its_widest_line() {
        let line = |name: &str, score: &str| (text(name), text(score));
        let l = lay(HudTexts {
            sidebar: Some(Arc::new(Sidebar {
                title: text("Top"),
                lines: vec![line("abcd", "12"), line("x", ""), line("y", "7")],
            })),
            ..HudTexts::default()
        });
        // "abcd", ": " and "12": 24 + 10 + 12.
        let (left, right) = (427 - 46 - 3, 426);
        let bottom = 120 + 27 / 3;
        assert_eq!(
            l.over(),
            [
                (
                    String::new(),
                    [left - 2, bottom - 37, right - left + 2, 9],
                    102 << 24
                ),
                (
                    String::new(),
                    [left - 2, bottom - 28, right - left + 2, 28],
                    76 << 24
                ),
            ]
        );
        assert_eq!(
            l.texts(),
            [
                ("Top".into(), [left + 23 - 9, bottom - 36], 1, WHITE),
                ("abcd".into(), [left, bottom - 27], 1, WHITE),
                ("12".into(), [right - 12, bottom - 27], 1, WHITE),
                ("x".into(), [left, bottom - 18], 1, WHITE),
                ("y".into(), [left, bottom - 9], 1, WHITE),
                ("7".into(), [right - 6, bottom - 9], 1, WHITE),
            ]
        );
        assert!(l.layout.texts().iter().all(|t| !t.shadow));
        // A line without a score has no room for one.
        let l = lay(HudTexts {
            sidebar: Some(Arc::new(Sidebar {
                title: text("T"),
                lines: vec![line("widest", ""), line("a", "1")],
            })),
            ..HudTexts::default()
        });
        assert_eq!(l.texts()[1].1[0], 427 - 36 - 3);
        // A title wider than its lines.
        let l = lay(HudTexts {
            sidebar: Some(Arc::new(Sidebar {
                title: text("A wide title"),
                lines: vec![line("a", "1")],
            })),
            ..HudTexts::default()
        });
        assert_eq!(l.texts()[1].1[0], 427 - 68 - 3);
    }

    fn player(id: u128, name: &str, latency: i32) -> ListedPlayer {
        ListedPlayer {
            id,
            name: text(name),
            spectator: false,
            latency,
            score: None,
            skin: Arc::new(SkinSource::Player { id, textures: None }),
            show_hat: true,
            flipped: false,
        }
    }

    fn list(players: Vec<ListedPlayer>, scores: ListScores) -> PlayerList {
        PlayerList {
            header: None,
            footer: None,
            players,
            scores,
            faces: false,
        }
    }

    fn lay_list(list: PlayerList) -> Laid {
        lay(HudTexts {
            players: Some(Arc::new(list)),
            ..HudTexts::default()
        })
    }

    #[test]
    fn the_player_list_has_a_row_for_each_player() {
        let mut spectator = player(2, "bb", 2000);
        spectator.spectator = true;
        let l = lay_list(list(
            vec![player(1, "aaaa", 20), spectator],
            ListScores::None,
        ));
        // 24 and 13 wide, about the middle.
        let left = 213 - 37 / 2;
        assert_eq!(
            l.over(),
            [
                (String::new(), [213 - 18 - 1, 9, 38, 19], LIST_GROUND),
                (String::new(), [left, 10, 37, 8], LIST_SLOT),
                ("icon/ping_5".into(), [left + 26, 10, 10, 8], WHITE),
                (String::new(), [left, 19, 37, 8], LIST_SLOT),
                ("icon/ping_1".into(), [left + 26, 19, 10, 8], WHITE),
            ]
        );
        assert_eq!(
            l.texts(),
            [
                ("aaaa".into(), [left, 10], 1, WHITE),
                ("bb".into(), [left, 19], 1, LIST_SPECTATOR),
            ]
        );
        let ping = |latency| {
            let l = lay_list(list(vec![player(1, "a", latency)], ListScores::None));
            l.over()[2].0.clone()
        };
        let pings = [-1, 0, 149, 150, 299, 300, 599, 600, 999, 1000].map(ping);
        let want = ["unknown", "5", "5", "4", "4", "3", "3", "2", "2", "1"];
        assert_eq!(pings.to_vec(), want.map(|n| format!("icon/ping_{n}")));
        // Nobody: the ground of no rows.
        assert_eq!(lay_list(list(Vec::new(), ListScores::None)).over().len(), 1);
    }

    #[test]
    fn the_player_list_stands_between_its_header_and_its_footer() {
        let l = lay_list(PlayerList {
            header: Some(text("The header\nb")),
            footer: Some(text("foot")),
            ..list(vec![player(1, "a", 0)], ListScores::None)
        });
        // The header's 58 wide; the names' ground is as wide as it.
        let sides = [213 - 29 - 1, 60];
        let grounds: Vec<[i32; 4]> = l
            .over()
            .iter()
            .filter(|q| q.2 == LIST_GROUND)
            .map(|q| q.1)
            .collect();
        assert_eq!(
            grounds,
            [
                [sides[0], 9, sides[1], 19],
                [sides[0], 28, sides[1], 10],
                [sides[0], 38, sides[1], 10],
            ]
        );
        let texts = l.texts();
        assert_eq!(texts[0], ("The header".into(), [213 - 29, 10], 1, WHITE));
        assert_eq!(texts[1], ("b".into(), [210, 19], 1, WHITE));
        assert_eq!(texts[2].1[1], 29);
        assert_eq!(texts[3], ("foot".into(), [213 - 12, 39], 1, WHITE));
    }

    #[test]
    fn more_than_twenty_players_stand_in_columns() {
        let many = |n: u128| -> Vec<ListedPlayer> { (0..n).map(|i| player(i, "abc", 0)).collect() };
        let l = lay_list(list(many(45), ListScores::None));
        // Three columns of fifteen, 31 wide each and five apart.
        let first = 213 - (31 * 3 + 10) / 2;
        let texts = l.texts();
        assert_eq!((texts[14].1, texts[15].1), ([first, 136], [first + 36, 10]));
        assert_eq!(texts[44].1, [first + 72, 136]);
        // No wider than the GUI less fifty.
        let wide = (0..80).map(|i| player(i, "a_very_long_name", 0)).collect();
        let l = lay_list(list(wide, ListScores::None));
        let slot = (427 - 50) / 4;
        assert_eq!(l.over()[1].1[2], slot);
        assert_eq!(l.texts()[20].1[0] - l.texts()[0].1[0], slot + 5);
    }

    #[test]
    fn faces_stand_before_the_names() {
        let mut upside_down = player(7, "ab", 0);
        (upside_down.flipped, upside_down.show_hat) = (true, false);
        let l = lay_list(PlayerList {
            faces: true,
            ..list(vec![upside_down], ListScores::None)
        });
        let left = 213 - (9 + 12 + 13) / 2;
        let face = &l.layout.faces[0];
        assert_eq!(
            (face.x, face.y, face.flipped, face.hat),
            (left, 10, true, false)
        );
        // Over the ground and the row, under the bars.
        assert_eq!(face.at, 2);
        assert_eq!(l.texts()[0].1, [left + 9, 10]);
        assert_eq!(l.over()[2].1[0], left + 34 - 11);
        assert!(lay(HudTexts::default()).layout.faces.is_empty());
    }

    fn scored(id: u128, name: &str, score: i32) -> ListedPlayer {
        ListedPlayer {
            score: Some((score, text(&score.to_string()))),
            ..player(id, name, 0)
        }
    }

    #[test]
    fn scores_stand_to_the_right_of_the_names() {
        let mut watching = scored(3, "c", 5);
        watching.spectator = true;
        let l = lay_list(list(
            vec![scored(1, "abc", 120), player(2, "b", 0), watching],
            ListScores::Numbers,
        ));
        // A space and "120": 22.
        let left = 213 - (18 + 22 + 13) / 2;
        let texts = l.texts();
        assert_eq!(
            texts[1],
            ("120".into(), [left + 18 + 1 + 22 - 18, 10], 1, WHITE)
        );
        assert_eq!(texts.len(), 4);
    }

    fn hearts_of(l: &Laid) -> Vec<(String, i32)> {
        l.over()
            .iter()
            .filter(|q| q.0.starts_with("hud/heart"))
            .map(|q| (q.0.trim_start_matches("hud/heart/").to_owned(), q.1[0]))
            .collect()
    }

    #[test]
    fn scores_can_be_hearts() {
        let l = lay_list(list(vec![scored(1, "abc", 5)], ListScores::Hearts));
        let left = 213 - (18 + 90 + 13) / 2 + 18 + 1;
        // Seven empty ones, then two whole and a half, eight apart (86 for ten).
        let hearts = hearts_of(&l);
        assert_eq!(hearts[0], ("container".into(), left + 24));
        assert_eq!(hearts[6], ("container".into(), left + 72));
        assert_eq!(
            hearts[7..],
            [
                ("container".into(), left),
                ("full".into(), left),
                ("container".into(), left + 8),
                ("full".into(), left + 8),
                ("container".into(), left + 16),
                ("half".into(), left + 16),
            ]
        );
        // What is over ten is golden.
        let l = lay_list(list(vec![scored(1, "abc", 23)], ListScores::Hearts));
        let last = hearts_of(&l);
        assert_eq!(last[last.len() - 1].0, "absorbing_half_blinking");
        assert_eq!(last[last.len() - 3].0, "absorbing_full_blinking");
        // Too many for their room are a number, from red to green.
        let l = lay_list(list(vec![scored(1, "abc", 60)], ListScores::Hearts));
        assert!(hearts_of(&l).is_empty());
        let t = l.texts()[1].clone();
        assert_eq!((t.0.as_str(), t.3), ("30.0 hp", 0xFF00_FF00));
        assert_eq!(t.1[0], (left + left + 90 - 40) / 2);
        // Nothing at all without a heart.
        let l = lay_list(list(vec![scored(1, "abc", 0)], ListScores::Hearts));
        assert!(hearts_of(&l).is_empty());
        let spectator = ListedPlayer {
            spectator: true,
            ..scored(1, "abc", 5)
        };
        let l = lay_list(list(vec![spectator], ListScores::Hearts));
        assert!(hearts_of(&l).is_empty());
    }

    #[test]
    fn the_lists_hearts_blink_when_they_change() {
        let mut memory = HudMemory::default();
        let mut at = |score, ticks, shown: bool| {
            let players = list(vec![scored(1, "abc", score)], ListScores::Hearts);
            let hud = HudRecord {
                ticks,
                texts: HudTexts {
                    players: shown.then(|| Arc::new(players)),
                    ..HudTexts::default()
                },
                ..HudRecord::default()
            };
            hearts_of(&lay_with(&hud, 0.0, Instant::now(), &mut memory))
        };
        let blinking = |h: &[(String, i32)]| h.iter().any(|(n, _)| n == "container_blinking");
        assert!(!blinking(&at(20, 100, true)));
        at(10, 101, true);
        let lost = at(10, 104, true);
        let lost = lost.iter().filter(|(n, _)| n == "full_blinking").count();
        assert_eq!(lost, 10);
        // Less: it blinks for 20 ticks, three of six on, and shows what was lost.
        let pattern: String = (101..122)
            .map(|t| if blinking(&at(10, t, true)) { '1' } else { '0' })
            .collect();
        assert_eq!(pattern, "000111000111000111000");
        // A second later what was lost is forgotten; more blinks for ten ticks only.
        assert!(
            !at(10, 130, true)
                .iter()
                .any(|(n, _)| n.contains("blinking"))
        );
        // Shown by what it blinks with when it changes again.
        at(9, 131, true);
        let blink = at(9, 134, true);
        let lost = blink.iter().filter(|(n, _)| n == "full_blinking").count();
        assert_eq!(lost, 5);
        assert!(!blinking(&at(9, 160, true)));
        at(14, 140, true);
        assert!((140..150).any(|t| blinking(&at(14, t, true))));
        assert!((150..170).all(|t| !blinking(&at(14, t, true))));
        // Hidden and shown again, it begins anew.
        at(14, 171, false);
        assert!((172..190).all(|t| !blinking(&at(2, t, true))));
    }

    #[test]
    fn bars_fill_as_time_goes_by() {
        // The frame's own time counts, not the layout's.
        use ruthenium_protocol::packets::play::BossEvent;
        use ruthenium_sim::hud_text::HudTextState;
        let start = Instant::now();
        let mut state = HudTextState::default();
        let add = BossEvent::Add {
            name: Tag::String("b".into()),
            progress: 0.0,
            color: 0,
            overlay: 0,
            flags: 0,
        };
        state.boss_event(1, add, start);
        state.boss_event(1, BossEvent::Progress(1.0), start);
        let mut hud = HudRecord::default();
        state.fill(
            &mut hud.texts,
            0,
            "",
            false,
            false,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        let width = |ms| {
            let now = start + Duration::from_millis(ms);
            let l = lay_with(&hud, 0.0, now, &mut HudMemory::default());
            l.over().get(1).map_or(0, |q| q.1[2])
        };
        assert_eq!([width(0), width(50), width(100)], [0, 91, 182]);
    }

    #[test]
    fn a_word_belongs_over_what_was_there_before_it() {
        // The sidebar's ground, the title, the chat's ground: each text between.
        let l = lay(HudTexts {
            sidebar: Some(Arc::new(Sidebar {
                title: text("s"),
                lines: Vec::new(),
            })),
            overlay: Some(Overlay {
                text: text("o"),
                time: 60,
                animated: false,
            }),
            selected: Some((text("n"), 40)),
            bosses: Arc::new(vec![BossBar::new(1, text("b"), 0.0, 0, 0)]),
            ..HudTexts::default()
        });
        let order: Vec<(String, usize)> = l
            .layout
            .texts()
            .iter()
            .map(|t| (shown(&t.line.as_ref().unwrap().chars), t.at))
            .collect();
        assert_eq!(
            order,
            [
                ("n".into(), 0),
                ("b".into(), 1),
                ("s".into(), 3),
                ("o".into(), 3)
            ]
        );
    }
}
