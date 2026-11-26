//! Small forms in the modern style: a title, optional message lines, an optional code
//! shown large, text fields with their labels, and buttons. Used for Add Server, Edit
//! Server, Direct Connect, signing in and confirmations.
//!
//! Controls take focus in order, fields first. A focused field shows its ring however it
//! got focus and takes typed text; buttons show theirs only after keyboard use.

use super::{
    Action, Button, ButtonStates, FAINT, Rect, SOFT, ShapeList, Text, UiScale, WHITE_TEXT,
    draw_button, draw_focus_ring, mix,
};
use crate::Clip;
use crate::text::{Fonts, GlyphAtlas, TextList, TextStyle, Weight};

const WIDTH: f32 = 440.0;
const FIELD_H: f32 = 48.0;
const FIELD_RADIUS: f32 = 14.0;
const FIELD_PAD: f32 = 16.0;
const LABEL_GAP: f32 = 24.0;
const FIELD_GAP: f32 = 20.0;
const BUTTON_H: f32 = 52.0;
const BUTTON_GAP: f32 = 12.0;
const TEXT_SIZE: f32 = 17.0;
/// A code shown large: its size and the height it takes.
const CODE_SIZE: f32 = 44.0;
const CODE_H: f32 = 72.0;
const CARET_W: f32 = 2.0;
/// Caret blink: on for this part of each second.
const BLINK_ON: f32 = 0.6;
const SELECTION: [f32; 4] = [0.45, 0.62, 1.0, 0.38];
const FIELD_TINT: [f32; 4] = [0.6, 0.66, 0.85, 0.10];
const FIELD_HOVER: [f32; 4] = [0.7, 0.76, 0.95, 0.16];

/// A field as a form starts with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
    /// The most UTF-16 units it takes (Java's string length, as vanilla counts).
    pub max: usize,
    /// Shown faintly while empty.
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormButton {
    pub label: String,
    pub action: Action,
}

/// What a form shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSpec {
    pub title: String,
    pub message: Vec<String>,
    /// A code to read off the screen (a sign-in code), large, under the message.
    pub code: Option<String>,
    pub fields: Vec<Field>,
    /// [`Action::Cycle`] buttons get a row each; the rest share the last row.
    pub buttons: Vec<FormButton>,
}

/// Editable text with a caret and a selection (byte offsets at character boundaries).
///
/// Typing filters out `§` and control characters, as vanilla's edit box does
/// (`StringUtil.filterText`), and stops at the length limit. Word steps (with the
/// shortcut modifier) skip to the next space and the spaces after it, or back to the
/// start of the previous word, as vanilla's `EditBox.getWordPosition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextField {
    value: String,
    caret: usize,
    anchor: usize,
    max: usize,
}

impl TextField {
    pub fn new(value: &str, max: usize) -> TextField {
        let mut f = TextField {
            value: String::new(),
            caret: 0,
            anchor: 0,
            max,
        };
        f.insert(value);
        f
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected range, start first (empty when nothing is selected).
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn selected_text(&self) -> &str {
        &self.value[self.selection()]
    }

    /// Replaces the selection with `text`, filtered and cut to fit.
    pub fn insert(&mut self, text: &str) {
        let sel = self.selection();
        let room = self.max.saturating_sub(
            self.value.encode_utf16().count() - self.value[sel.clone()].encode_utf16().count(),
        );
        let mut used = 0;
        let filtered: String = text
            .chars()
            .filter(|&c| c != '§' && c >= ' ' && c != '\u{7F}')
            .take_while(|c| {
                used += c.len_utf16();
                used <= room
            })
            .collect();
        self.value.replace_range(sel.clone(), &filtered);
        self.caret = sel.start + filtered.len();
        self.anchor = self.caret;
    }

    /// Backspace (`word`: back to the previous word's start). A selection is deleted.
    pub fn backspace(&mut self, word: bool) {
        if self.caret == self.anchor {
            self.anchor = if word {
                self.word_left(self.caret)
            } else {
                self.char_left(self.caret)
            };
        }
        self.insert("");
    }

    /// Delete (`word`: to the next word). A selection is deleted.
    pub fn delete(&mut self, word: bool) {
        if self.caret == self.anchor {
            self.anchor = if word {
                self.word_right(self.caret)
            } else {
                self.char_right(self.caret)
            };
        }
        self.insert("");
    }

    /// Moves the caret left or right by a character or word; `select` extends the
    /// selection, otherwise a selection collapses to its edge.
    pub fn step(&mut self, right: bool, word: bool, select: bool) {
        let to = match (right, word) {
            _ if !select && self.caret != self.anchor && !word => {
                let sel = self.selection();
                if right { sel.end } else { sel.start }
            }
            (true, true) => self.word_right(self.caret),
            (true, false) => self.char_right(self.caret),
            (false, true) => self.word_left(self.caret),
            (false, false) => self.char_left(self.caret),
        };
        self.move_to(to, select);
    }

    /// Moves the caret to `at` (a byte offset, snapped to a character boundary).
    pub fn move_to(&mut self, at: usize, select: bool) {
        let mut at = at.min(self.value.len());
        while !self.value.is_char_boundary(at) {
            at -= 1;
        }
        self.caret = at;
        if !select {
            self.anchor = at;
        }
    }

    pub fn home(&mut self, select: bool) {
        self.move_to(0, select);
    }

    pub fn end(&mut self, select: bool) {
        self.move_to(self.value.len(), select);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.value.len();
    }

    fn char_left(&self, at: usize) -> usize {
        self.value[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn char_right(&self, at: usize) -> usize {
        self.value[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }

    fn word_left(&self, at: usize) -> usize {
        let b = self.value.as_bytes();
        let mut i = at;
        while i > 0 && b[i - 1] == b' ' {
            i -= 1;
        }
        while i > 0 && b[i - 1] != b' ' {
            i -= 1;
        }
        i
    }

    fn word_right(&self, at: usize) -> usize {
        let b = self.value.as_bytes();
        match self.value[at..].find(' ') {
            None => b.len(),
            Some(off) => {
                let mut i = at + off;
                while i < b.len() && b[i] == b' ' {
                    i += 1;
                }
                i
            }
        }
    }
}

#[derive(Debug, Clone)]
struct FieldView {
    rect: Rect,
    label: Text,
    hint: Text,
    field: TextField,
    /// The value shaped, and the pen position (pixels) of every character boundary.
    shaped: Text,
    stops: Vec<(usize, f32)>,
    /// Horizontal scroll in pixels, keeping the caret in view.
    scroll: f32,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct FormScreen {
    pub scale: UiScale,
    pub spec: FormSpec,
    pub buttons: Vec<Button>,
    pub states: ButtonStates,
    title: Text,
    message: Vec<(Text, Rect)>,
    code: Option<(Text, Rect)>,
    title_rect: Rect,
    fields: Vec<FieldView>,
    labels: Vec<Text>,
    blink: f32,
    /// At most this many message lines show at once, the rest scrolled to
    /// ([`FormScreen::set_message_window`]); the first line shown.
    window: Option<usize>,
    scroll: usize,
    /// Where the shown lines are, while they are windowed.
    message_view: Option<Rect>,
}

/// The message lines' size.
const MESSAGE_SIZE: f32 = 17.0;
const MESSAGE_STYLE: TextStyle = TextStyle {
    size: MESSAGE_SIZE,
    weight: Weight::REGULAR,
    tracking: 0.0,
};

impl TextStyle {
    /// At `px` pixels per UI unit.
    fn with_px(self, px: f32) -> TextStyle {
        TextStyle {
            size: self.size * px,
            ..self
        }
    }
}

/// `text` in lines no wider than `max` pixels, broken after spaces where it can be and
/// between characters where a word is wider than a line (text without spaces, as Chinese
/// and Japanese are written).
fn wrap_to(fonts: &Fonts, text: &str, style: TextStyle, max: f32) -> Vec<String> {
    let width = |s: &str| fonts.shape(s, style).width;
    if width(text) <= max {
        return vec![text.to_owned()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_inclusive(' ') {
        if width(format!("{line}{word}").trim_end()) <= max {
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(line.trim_end().to_owned());
            line.clear();
        }
        for c in word.chars() {
            line.push(c);
            // Closing punctuation never starts a line; it may overhang this one.
            if width(line.trim_end()) > max && line.chars().count() > 1 && !closing(c) {
                line.pop();
                lines.push(std::mem::take(&mut line));
                line.push(c);
            }
        }
    }
    if !line.trim_end().is_empty() {
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// Punctuation that closes what comes before it (a line doesn't start with it).
fn closing(c: char) -> bool {
    "、。，．！？：；）」』】〉》〕”’…ー.,!?:;)]}".contains(c)
}

fn shape_value(fonts: &Fonts, scale: &UiScale, value: &str) -> (Text, Vec<(usize, f32)>) {
    let style = TextStyle {
        size: TEXT_SIZE * scale.px,
        weight: Weight::REGULAR,
        tracking: 0.0,
    };
    let stops = value
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(value.len()))
        .map(|i| {
            (
                i,
                if i == 0 {
                    0.0
                } else {
                    fonts.shape(&value[..i], style).width
                },
            )
        })
        .collect();
    (
        Text::new(fonts, value, scale, TEXT_SIZE, Weight::REGULAR, 0.0),
        stops,
    )
}

impl FormScreen {
    pub fn new(scale: UiScale, fonts: &Fonts, spec: FormSpec) -> FormScreen {
        let n = spec.fields.len() + spec.buttons.len();
        let fields = spec
            .fields
            .iter()
            .map(|f| {
                let field = TextField::new(&f.value, f.max);
                let (shaped, stops) = shape_value(fonts, &scale, field.value());
                FieldView {
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    },
                    label: Text::new(fonts, &f.label, &scale, 14.0, Weight::MEDIUM, 0.01),
                    hint: Text::new(fonts, &f.hint, &scale, TEXT_SIZE, Weight::REGULAR, 0.0),
                    field,
                    shaped,
                    stops,
                    scroll: 0.0,
                }
            })
            .collect();
        let mut form = FormScreen {
            scale,
            buttons: spec
                .buttons
                .iter()
                .map(|b| Button {
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    },
                    label: "",
                    action: b.action,
                })
                .collect(),
            states: ButtonStates::new(n),
            title: Text::new(fonts, &spec.title, &scale, 26.0, Weight::SEMIBOLD, 0.01),
            message: Vec::new(),
            code: None,
            title_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            fields,
            labels: Vec::new(),
            blink: 0.0,
            window: None,
            scroll: 0,
            message_view: None,
            spec,
        };
        if !form.fields.is_empty() {
            form.states.set_focus(Some(0), false);
        }
        form.layout(scale, fonts);
        form
    }

    /// Lays the form out for `scale`, keeping what was typed.
    pub fn layout(&mut self, scale: UiScale, fonts: &Fonts) {
        self.scale = scale;
        let cycles = self
            .spec
            .buttons
            .iter()
            .filter(|b| b.action == Action::Cycle)
            .count();
        let last_row = !self.spec.buttons.is_empty() && cycles < self.spec.buttons.len();
        let rows = cycles + usize::from(last_row);
        let lines: Vec<String> = self
            .spec
            .message
            .iter()
            .flat_map(|m| wrap_to(fonts, m, MESSAGE_STYLE.with_px(scale.px), WIDTH * scale.px))
            .collect();
        let shown = self.window.map_or(lines.len(), |w| lines.len().min(w));
        self.scroll = self.scroll.min(lines.len() - shown);
        let msg_h = shown as f32 * 26.0;
        let code_h = if self.spec.code.is_some() {
            CODE_H
        } else {
            0.0
        };
        let height = 44.0
            + if msg_h > 0.0 { 18.0 + msg_h } else { 0.0 }
            + code_h
            + 30.0
            + self.fields.len() as f32 * (LABEL_GAP + FIELD_H + FIELD_GAP)
            + 10.0
            + rows as f32 * (BUTTON_H + BUTTON_GAP);
        let x = 0.5 * scale.width - 0.5 * WIDTH;
        let mut y = ((scale.height - height) * 0.42).max(24.0);
        self.title = Text::new(
            fonts,
            &self.spec.title,
            &scale,
            26.0,
            Weight::SEMIBOLD,
            0.01,
        );
        self.title_rect = Rect {
            x,
            y,
            w: WIDTH,
            h: 44.0,
        };
        y += 44.0;
        if msg_h > 0.0 {
            y += 18.0;
        }
        self.message_view = (shown < lines.len()).then_some(Rect {
            x,
            y,
            w: WIDTH,
            h: msg_h,
        });
        let message_end = y + msg_h;
        self.message = lines
            .iter()
            .map(|m| {
                let t = Text::new(fonts, m, &scale, MESSAGE_SIZE, Weight::REGULAR, 0.0);
                let r = Rect {
                    x,
                    y,
                    w: WIDTH,
                    h: 26.0,
                };
                y += 26.0;
                (t, r)
            })
            .collect();
        y = message_end;
        self.code = self.spec.code.as_ref().map(|c| {
            let t = Text::new(fonts, c, &scale, CODE_SIZE, Weight::EXTRABOLD, 0.14);
            let r = Rect {
                x,
                y: y + 8.0,
                w: WIDTH,
                h: CODE_H - 8.0,
            };
            y += CODE_H;
            (t, r)
        });
        y += 30.0;
        for (f, spec) in self.fields.iter_mut().zip(&self.spec.fields) {
            f.label = Text::new(fonts, &spec.label, &scale, 14.0, Weight::MEDIUM, 0.01);
            f.hint = Text::new(fonts, &spec.hint, &scale, TEXT_SIZE, Weight::REGULAR, 0.0);
            (f.shaped, f.stops) = shape_value(fonts, &scale, f.field.value());
            f.rect = Rect {
                x,
                y: y + LABEL_GAP,
                w: WIDTH,
                h: FIELD_H,
            };
            y += LABEL_GAP + FIELD_H + FIELD_GAP;
        }
        y += 10.0;
        let shared: Vec<usize> = (0..self.buttons.len())
            .filter(|&i| self.buttons[i].action != Action::Cycle)
            .collect();
        for i in 0..self.buttons.len() {
            if self.buttons[i].action == Action::Cycle {
                self.buttons[i].rect = Rect {
                    x,
                    y,
                    w: WIDTH,
                    h: BUTTON_H,
                };
                y += BUTTON_H + BUTTON_GAP;
            }
        }
        let n = shared.len().max(1) as f32;
        let w = (WIDTH - (n - 1.0) * BUTTON_GAP) / n;
        for (k, &i) in shared.iter().enumerate() {
            self.buttons[i].rect = Rect {
                x: x + k as f32 * (w + BUTTON_GAP),
                y,
                w,
                h: BUTTON_H,
            };
        }
        self.labels = self
            .spec
            .buttons
            .iter()
            .map(|b| Text::new(fonts, &b.label, &scale, 18.0, Weight::SEMIBOLD, 0.01))
            .collect();
        for i in 0..self.fields.len() {
            self.keep_caret_visible(i);
        }
    }

    pub fn field(&self, i: usize) -> &TextField {
        &self.fields[i].field
    }

    /// Shows at most `lines` lines of the message at once; the wheel scrolls the rest
    /// ([`FormScreen::wheel`]).
    pub fn set_message_window(&mut self, lines: usize, fonts: &Fonts) {
        self.window = Some(lines.max(1));
        self.layout(self.scale, fonts);
    }

    /// The wheel turned `notches` (positive: up) over the form: a windowed message
    /// scrolls a line a notch.
    pub fn wheel(&mut self, notches: f32) {
        let Some(shown) = self.window else { return };
        let most = self.message.len().saturating_sub(shown);
        let by = notches.round() as i64;
        self.scroll = (self.scroll as i64 - by).clamp(0, most as i64) as usize;
    }

    /// The first message line shown, and how many there are.
    pub fn message_scroll(&self) -> (usize, usize) {
        (self.scroll, self.message.len())
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Enables or disables button `i` (counted among the buttons only).
    pub fn set_button_enabled(&mut self, i: usize, on: bool) {
        self.states.set_enabled(self.fields.len() + i, on);
    }

    pub fn button_enabled(&self, i: usize) -> bool {
        self.states.enabled(self.fields.len() + i)
    }

    /// Relabels button `i` (a choice button showing its new value).
    pub fn set_button_label(&mut self, i: usize, fonts: &Fonts, label: &str) {
        self.spec.buttons[i].label = label.to_owned();
        self.labels[i] = Text::new(fonts, label, &self.scale, 18.0, Weight::SEMIBOLD, 0.01);
    }

    /// The focused field, if focus is on one.
    pub fn focused_field(&self) -> Option<usize> {
        self.states.focused().filter(|&i| i < self.fields.len())
    }

    /// The control (fields, then buttons) at `(x, y)` (UI units).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| f.rect.contains(x, y))
            .or_else(|| {
                self.buttons
                    .iter()
                    .position(|b| b.rect.contains(x, y))
                    .map(|i| i + self.fields.len())
            })
    }

    /// The left button went down at `(x, y)`; a click in a field places the caret
    /// (`select` with Shift extends the selection).
    pub fn mouse_down(&mut self, x: f32, y: f32, select: bool) {
        let hit = self.hit(x, y);
        self.states.mouse_down(hit);
        if let Some(i) = hit.filter(|&i| i < self.fields.len()) {
            let f = &mut self.fields[i];
            let px = (x - f.rect.x - FIELD_PAD) * self.scale.px + f.scroll;
            let at = f
                .stops
                .iter()
                .min_by(|a, b| (a.1 - px).abs().total_cmp(&(b.1 - px).abs()))
                .map_or(0, |s| s.0);
            f.field.move_to(at, select);
            self.blink = 0.0;
        }
    }

    /// The left button went up at `(x, y)`: the action of the button released over.
    pub fn mouse_up(&mut self, x: f32, y: f32) -> Option<Action> {
        let i = self.states.mouse_up(self.hit(x, y))?;
        let b = i.checked_sub(self.fields.len())?;
        Some(self.buttons[b].action)
    }

    /// Edits the focused field with `edit`, then shapes it again.
    pub fn edit(&mut self, fonts: &Fonts, edit: impl FnOnce(&mut TextField)) {
        let Some(i) = self.focused_field() else {
            return;
        };
        edit(&mut self.fields[i].field);
        let f = &mut self.fields[i];
        (f.shaped, f.stops) = shape_value(fonts, &self.scale, f.field.value());
        self.blink = 0.0;
        self.keep_caret_visible(i);
    }

    fn keep_caret_visible(&mut self, i: usize) {
        let s = self.scale;
        let f = &mut self.fields[i];
        let inner = (f.rect.w - 2.0 * FIELD_PAD) * s.px;
        let caret = f
            .stops
            .iter()
            .find(|st| st.0 == f.field.caret())
            .map_or(0.0, |st| st.1);
        let total = f.stops.last().map_or(0.0, |st| st.1);
        if caret - f.scroll > inner {
            f.scroll = caret - inner;
        }
        if caret < f.scroll {
            f.scroll = caret;
        }
        f.scroll = f.scroll.min((total - inner).max(0.0)).max(0.0);
    }

    /// Advances the caret's blink and the buttons' motion.
    pub fn update(&mut self, dt: f32) {
        self.blink = (self.blink + dt.max(0.0)) % 1.0;
        self.states.update(dt);
    }

    pub fn draw(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: Option<&GlyphAtlas>) {
        let s = &self.scale;
        let nf = self.fields.len();
        if let Some(atlas) = atlas {
            self.title
                .centred(text, atlas, s, self.title_rect, WHITE_TEXT);
            match self.message_view {
                None => {
                    for (t, r) in &self.message {
                        t.centred(text, atlas, s, *r, SOFT);
                    }
                }
                Some(view) => {
                    // The lines scrolled to, in the message's place.
                    text.set_clip(Clip::rect(super::px_rect(s, view)));
                    let up = self.scroll as f32 * 26.0;
                    for (t, r) in &self.message {
                        let r = Rect { y: r.y - up, ..*r };
                        if r.y + r.h > view.y && r.y < view.y + view.h {
                            t.centred(text, atlas, s, r, SOFT);
                        }
                    }
                    text.set_clip(Clip::NONE);
                }
            }
            if let Some((t, r)) = &self.code {
                t.centred(text, atlas, s, *r, WHITE_TEXT);
            }
        }
        // Where a scrolled message is: a thin bar beside it.
        if let Some(view) = self.message_view {
            let total = self.message.len().max(1) as f32;
            let shown = view.h / 26.0;
            let bar = Rect {
                x: view.x + view.w + 8.0,
                y: view.y + view.h * self.scroll as f32 / total,
                w: 3.0,
                h: (view.h * shown / total).max(8.0),
            };
            shapes.rounded(s, bar, 1.5, [0.85, 0.87, 1.0, 0.35]);
        }
        for (i, f) in self.fields.iter().enumerate() {
            let focused = self.states.focused() == Some(i);
            let look = self.states.look(i);
            let tint = mix(
                FIELD_TINT,
                FIELD_HOVER,
                look.hover.max(f32::from(u8::from(focused))),
            );
            shapes.glass_rect(s, f.rect, FIELD_RADIUS, tint, 0.6);
            draw_focus_ring(
                shapes,
                s,
                f.rect,
                FIELD_RADIUS,
                if focused { 1.0 } else { look.focus },
            );
            let inner = Rect {
                x: f.rect.x + FIELD_PAD - 2.0,
                y: f.rect.y,
                w: f.rect.w - 2.0 * FIELD_PAD + 4.0,
                h: f.rect.h,
            };
            let clip = Clip::rect(super::px_rect(s, inner));
            let x0 = (f.rect.x + FIELD_PAD) * s.px - f.scroll;
            let cy = (f.rect.y + 0.5 * f.rect.h) * s.px;
            let pos = |at: usize| f.stops.iter().find(|st| st.0 == at).map_or(0.0, |st| st.1);
            shapes.set_clip(clip);
            let sel = f.field.selection();
            if focused && !sel.is_empty() {
                let (a, b) = (x0 + pos(sel.start), x0 + pos(sel.end));
                let h = 24.0 * s.px;
                shapes.rounded(
                    s,
                    Rect {
                        x: a / s.px,
                        y: (cy - 0.5 * h) / s.px,
                        w: (b - a) / s.px,
                        h: h / s.px,
                    },
                    3.0,
                    SELECTION,
                );
            }
            if focused && self.blink < BLINK_ON {
                let x = x0 + pos(f.field.caret());
                let h = 22.0 * s.px;
                shapes.rounded(
                    s,
                    Rect {
                        x: x / s.px - 0.5 * CARET_W,
                        y: (cy - 0.5 * h) / s.px,
                        w: CARET_W,
                        h: h / s.px,
                    },
                    1.0,
                    WHITE_TEXT,
                );
            }
            shapes.set_clip(Clip::NONE);
            let Some(atlas) = atlas else { continue };
            f.label
                .draw_at(text, atlas, s, f.rect.x + 4.0, f.rect.y - 9.0, FAINT);
            text.set_clip(clip);
            if f.field.value().is_empty() {
                f.hint.left_in(
                    text,
                    atlas,
                    s,
                    Rect {
                        x: f.rect.x + FIELD_PAD,
                        ..f.rect
                    },
                    [0.85, 0.87, 1.0, 0.3],
                );
            } else {
                text.line(
                    atlas,
                    &f.shaped.line,
                    x0,
                    cy + 0.5 * f.shaped.cap,
                    WHITE_TEXT,
                );
            }
            text.set_clip(Clip::NONE);
        }
        for (i, (b, label)) in self.buttons.iter().zip(&self.labels).enumerate() {
            let c = nf + i;
            draw_button(
                shapes,
                text,
                atlas,
                s,
                b.rect,
                label,
                self.states.look(c),
                self.states.enabled(c),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modern::WEIGHTS;

    #[test]
    fn typing_filters_and_stops_at_the_limit() {
        let mut f = TextField::new("", 5);
        f.insert("a§b\tc");
        assert_eq!(f.value(), "abc");
        f.insert("defgh");
        assert_eq!(f.value(), "abcde");
        // A clef is two UTF-16 units and does not fit in one.
        let mut f = TextField::new("abcd", 5);
        f.insert("𝄞");
        assert_eq!(f.value(), "abcd");
    }

    #[test]
    fn caret_moves_and_selection_edits() {
        let mut f = TextField::new("play.example.org", 128);
        assert_eq!(f.caret(), 16);
        f.step(false, false, true);
        f.step(false, false, true);
        assert_eq!(f.selected_text(), "rg");
        f.insert("net");
        assert_eq!(f.value(), "play.example.onet");
        f.select_all();
        f.backspace(false);
        assert_eq!(f.value(), "");
        let mut f = TextField::new("héllo wörld  again", 128);
        f.home(false);
        f.step(true, true, false);
        assert_eq!(f.caret(), "héllo ".len());
        f.step(true, true, false);
        assert_eq!(f.caret(), "héllo wörld  ".len());
        f.backspace(true);
        assert_eq!(f.value(), "héllo again");
        f.step(false, false, false);
        f.delete(false);
        assert_eq!(f.value(), "hélloagain");
        f.home(false);
        f.step(true, false, false);
        f.step(true, false, false);
        assert_eq!(f.caret(), "hé".len());
    }

    #[test]
    fn form_lays_out_and_focuses_the_first_field() {
        let fonts = Fonts::new(&WEIGHTS);
        let spec = FormSpec {
            title: "Add Server".into(),
            message: vec![],
            code: None,
            fields: vec![
                Field {
                    label: "Server Name".into(),
                    value: "".into(),
                    max: 32,
                    hint: "Minecraft Server".into(),
                },
                Field {
                    label: "Server Address".into(),
                    value: "".into(),
                    max: 128,
                    hint: "".into(),
                },
            ],
            buttons: vec![
                FormButton {
                    label: "Resource Packs: Prompt".into(),
                    action: Action::Cycle,
                },
                FormButton {
                    label: "Done".into(),
                    action: Action::Confirm,
                },
                FormButton {
                    label: "Cancel".into(),
                    action: Action::Cancel,
                },
            ],
        };
        let mut form = FormScreen::new(UiScale::new(1600, 900), &fonts, spec);
        assert_eq!(form.focused_field(), Some(0));
        form.edit(&fonts, |f| f.insert("My server"));
        assert_eq!(form.field(0).value(), "My server");
        // The cycle button spans the column; Done and Cancel share a row below it.
        let [cycle, done, cancel] = [0, 1, 2].map(|i| form.buttons[i].rect);
        assert_eq!(cycle.w, WIDTH);
        assert!(done.y > cycle.y && done.y == cancel.y && cancel.x > done.x);
        // Clicking the address field focuses it; releasing on Done acts.
        let addr = form.fields[1].rect;
        form.mouse_down(addr.x + 5.0, addr.y + 5.0, false);
        assert_eq!(form.focused_field(), Some(1));
        form.mouse_down(done.x + 5.0, done.y + 5.0, false);
        assert_eq!(
            form.mouse_up(done.x + 5.0, done.y + 5.0),
            Some(Action::Confirm)
        );
        form.set_button_enabled(1, false);
        form.mouse_down(done.x + 5.0, done.y + 5.0, false);
        assert_eq!(form.mouse_up(done.x + 5.0, done.y + 5.0), None);
    }

    #[test]
    fn messages_wrap_to_the_form_by_width() {
        let fonts = Fonts::new(&WEIGHTS);
        let style = MESSAGE_STYLE.with_px(1.0);
        let w = |s: &str| fonts.shape(s, style).width;
        assert_eq!(wrap_to(&fonts, "Short.", style, 440.0), ["Short."]);
        let long = "You were kicked from the server because the server is restarting now";
        let lines = wrap_to(&fonts, long, style, 300.0);
        assert!(
            lines.len() > 1 && lines.iter().all(|l| w(l) <= 300.0),
            "{lines:?}"
        );
        assert_eq!(lines.join(" "), long);
        // Text without spaces breaks between characters.
        let cjk = "服务器正在重新启动请稍后再试服务器正在重新启动请稍后再试服务器正在重新启动";
        let lines = wrap_to(&fonts, cjk, style, 200.0);
        assert!(
            lines.len() > 1 && lines.iter().all(|l| w(l) <= 200.0),
            "{lines:?}"
        );
        assert_eq!(lines.concat(), cjk);
        assert!(
            lines.iter().skip(1).all(|l| !l.starts_with('。')),
            "{lines:?}"
        );
        let lines = wrap_to(&fonts, "一二三四五。六", style, w("一二三四五"));
        assert_eq!(lines, ["一二三四五。", "六"]);
    }

    /// A long message shows a window of its lines, scrolled by the wheel, within its
    /// ends; the form's height follows the window, not the message.
    #[test]
    fn a_windowed_message_scrolls_within_its_lines() {
        let fonts = Fonts::new(&WEIGHTS);
        let spec = |lines: usize| FormSpec {
            title: "Prompt".into(),
            message: (0..lines).map(|i| format!("line {i}")).collect(),
            code: None,
            fields: Vec::new(),
            buttons: vec![FormButton {
                label: "Yes".into(),
                action: Action::Confirm,
            }],
        };
        let scale = UiScale::new(1600, 900);
        let mut long = FormScreen::new(scale, &fonts, spec(40));
        long.set_message_window(5, &fonts);
        let mut short = FormScreen::new(scale, &fonts, spec(5));
        short.set_message_window(5, &fonts);
        assert_eq!(long.buttons[0].rect.y, short.buttons[0].rect.y);
        assert_eq!(long.message_scroll(), (0, 40));
        long.wheel(-3.0);
        assert_eq!(long.message_scroll(), (3, 40));
        long.wheel(-100.0);
        assert_eq!(long.message_scroll(), (35, 40));
        long.wheel(1000.0);
        assert_eq!(long.message_scroll(), (0, 40));
        // A message that fits doesn't scroll.
        short.wheel(-3.0);
        assert_eq!(short.message_scroll(), (0, 5));
    }
}
