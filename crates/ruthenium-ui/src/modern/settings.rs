//! The Settings screen: settings in grouped glass cards, one row each with a label, a
//! line saying what it does, and a control on the right: a switch, a slider over fixed
//! steps, or a segmented choice. Done at the bottom.
//!
//! Rows take focus like buttons (Up/Down, Tab); Left/Right change the focused setting,
//! Enter/Space flip a switch or step a choice. A click flips a switch anywhere on its
//! row and picks the segment under the pointer; a slider follows the pointer from the
//! moment it goes down on its track.
//!
//! The screen keeps layout, values, scrolling, focus and motion; the client keeps the
//! settings themselves, applies each [`SettingsEvent::Changed`] and saves.

use super::{
    Action, Button, ButtonStates, Ease, FAINT, Key, Rect, SOFT, ShapeList, Text, UiScale,
    WHITE_TEXT, draw_button, draw_focus_ring, mix, px_rect,
};
use crate::Clip;
use crate::text::{Fonts, GlyphAtlas, TextList, Weight};

/// What a setting's control is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlSpec {
    /// On (1) or off (0).
    Switch,
    /// A slider over these steps, the step's label beside it.
    Slider(Vec<String>),
    /// One of these, side by side.
    Choice(Vec<String>),
}

/// One setting as the screen starts with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSpec {
    pub label: String,
    /// A short line under the label.
    pub detail: String,
    pub control: ControlSpec,
    /// The switch's state (0 or 1), the slider's step or the choice's index.
    pub value: usize,
}

/// A titled group of settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSpec {
    pub title: String,
    pub settings: Vec<SettingSpec>,
}

/// What the player did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEvent {
    /// Setting `setting` (counted over every section, in order) now has `value`.
    Changed { setting: usize, value: usize },
    /// Done, or Escape.
    Done,
}

const WIDTH: f32 = 620.0;
const TOP: f32 = 104.0;
const ROW_H: f32 = 68.0;
const PAD: f32 = 22.0;
const CARD_RADIUS: f32 = 20.0;
const HEADER_H: f32 = 34.0;
const SECTION_GAP: f32 = 22.0;
const BUTTON_W: f32 = 240.0;
const BUTTON_H: f32 = 44.0;
const FOOTER_MARGIN: f32 = 30.0;
/// Room between the cards and Done.
const FOOTER_GAP: f32 = 24.0;
/// Space kept between the text and the control.
const CONTROL_GAP: f32 = 18.0;

const SWITCH_W: f32 = 50.0;
const SWITCH_H: f32 = 30.0;
const KNOB_INSET: f32 = 3.0;
const TRACK_W: f32 = 190.0;
const TRACK_H: f32 = 6.0;
const SLIDER_KNOB: f32 = 22.0;
const VALUE_W: f32 = 84.0;
/// How far beside the track a press still grabs the slider.
const GRAB: f32 = 14.0;
const SEGMENT_H: f32 = 34.0;
const SEGMENT_MIN: f32 = 64.0;
const SEGMENT_PAD: f32 = 28.0;

/// Units per wheel notch, the scroll glide and how far the edges fade (as the lists).
const WHEEL_STEP: f32 = 64.0;
const SCROLL_TAU: f32 = 0.07;
const FADE: f32 = 28.0;
/// Time constant of a switch's knob, a choice's highlight and a slider stepped from the
/// keyboard.
const SLIDE_TAU: f32 = 0.08;

const CARD_TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.035];
const ROW_HOVER: [f32; 4] = [0.92, 0.94, 1.0, 0.05];
const HAIRLINE: [f32; 4] = [0.85, 0.88, 1.0, 0.09];
const ACCENT: [f32; 4] = [0.45, 0.62, 1.0, 0.92];
const TRACK_OFF: [f32; 4] = [0.75, 0.8, 1.0, 0.16];
const KNOB: [f32; 4] = [0.97, 0.98, 1.0, 1.0];
const SEGMENTS: [f32; 4] = [0.6, 0.66, 0.85, 0.12];
const SEGMENT_ON: [f32; 4] = [0.88, 0.92, 1.0, 0.24];
const SEGMENT_HOVER: [f32; 4] = [0.88, 0.92, 1.0, 0.07];

#[derive(Debug, Clone)]
enum Control {
    Switch,
    Slider(Vec<Text>),
    Choice(Vec<Text>),
}

impl Control {
    fn steps(&self) -> usize {
        match self {
            Control::Switch => 2,
            Control::Slider(t) | Control::Choice(t) => t.len().max(1),
        }
    }
}

#[derive(Debug, Clone)]
struct Row {
    label: Text,
    detail: Text,
    control: Control,
    value: usize,
    /// Where the knob or highlight shows, in steps (eased towards `value`).
    shown: Ease,
    /// Top of the row in the content (UI units, before scrolling).
    y: f32,
    /// The card's first and last row.
    first: bool,
    last: bool,
}

#[derive(Debug, Clone)]
struct Header {
    text: Text,
    y: f32,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct SettingsScreen {
    pub scale: UiScale,
    pub sections: Vec<SectionSpec>,
    /// Rows, then Done.
    pub states: ButtonStates,
    done: Button,
    done_label: Text,
    title: Text,
    headers: Vec<Header>,
    rows: Vec<Row>,
    /// Where the cards show (UI units).
    area: Rect,
    content_h: f32,
    scroll: f32,
    scroll_target: f32,
    /// The slider being dragged.
    dragging: Option<usize>,
    pointer: Option<(f32, f32)>,
    still: bool,
}

impl SettingsScreen {
    pub fn new(scale: UiScale, fonts: &Fonts, sections: Vec<SectionSpec>) -> SettingsScreen {
        let n: usize = sections.iter().map(|s| s.settings.len()).sum();
        let blank = || Text::new(fonts, "", &scale, 12.0, Weight::REGULAR, 0.0);
        let mut screen = SettingsScreen {
            scale,
            states: ButtonStates::new(n + 1),
            done: Button {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                },
                label: "Done",
                action: Action::Back,
            },
            done_label: blank(),
            title: blank(),
            headers: Vec::new(),
            rows: Vec::new(),
            area: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            content_h: 0.0,
            scroll: 0.0,
            scroll_target: 0.0,
            dragging: None,
            pointer: None,
            still: false,
            sections,
        };
        screen.layout(scale, fonts);
        for r in &mut screen.rows {
            r.shown.value = r.value as f32;
        }
        screen
    }

    /// How many settings it shows (Done is control number `len()`).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Lays the screen out for `scale`, keeping values, focus and scrolling.
    pub fn layout(&mut self, scale: UiScale, fonts: &Fonts) {
        self.scale = scale;
        let s = &scale;
        let w = WIDTH.min(scale.width - 48.0).max(360.0);
        let cx = 0.5 * scale.width;
        let y0 = scale.height - FOOTER_MARGIN - BUTTON_H;
        self.done.rect = Rect {
            x: cx - 0.5 * BUTTON_W,
            y: y0,
            w: BUTTON_W,
            h: BUTTON_H,
        };
        self.done_label = Text::new(fonts, "Done", s, 16.0, Weight::SEMIBOLD, 0.01);
        self.title = Text::new(fonts, "Settings", s, 28.0, Weight::SEMIBOLD, 0.01);
        self.area = Rect {
            x: cx - 0.5 * w,
            y: TOP,
            w,
            h: (y0 - FOOTER_GAP - TOP).max(ROW_H),
        };
        let old = std::mem::take(&mut self.rows);
        self.headers.clear();
        let mut y = 0.0;
        for (k, section) in self.sections.iter().enumerate() {
            if k > 0 {
                y += SECTION_GAP;
            }
            self.headers.push(Header {
                text: Text::new(
                    fonts,
                    &section.title.to_uppercase(),
                    s,
                    12.0,
                    Weight::EXTRABOLD,
                    0.08,
                ),
                y,
            });
            y += HEADER_H;
            let count = section.settings.len();
            for (j, spec) in section.settings.iter().enumerate() {
                let i = self.rows.len();
                let texts = |t: &[String]| {
                    t.iter()
                        .map(|l| Text::new(fonts, l, s, 14.5, Weight::MEDIUM, 0.0))
                        .collect()
                };
                let control = match &spec.control {
                    ControlSpec::Switch => Control::Switch,
                    ControlSpec::Slider(l) => Control::Slider(texts(l)),
                    ControlSpec::Choice(l) => Control::Choice(texts(l)),
                };
                let (value, shown) = old
                    .get(i)
                    .map_or((spec.value, Ease::default()), |r| (r.value, r.shown));
                self.rows.push(Row {
                    label: Text::new(fonts, &spec.label, s, 17.0, Weight::SEMIBOLD, 0.0),
                    detail: Text::new(fonts, &spec.detail, s, 13.5, Weight::REGULAR, 0.0),
                    value: value.min(control.steps() - 1),
                    control,
                    shown,
                    y,
                    first: j == 0,
                    last: j + 1 == count,
                });
                y += ROW_H;
            }
        }
        self.content_h = y;
        self.clamp_scroll();
        self.scroll = self.scroll_target;
    }

    /// Reduced motion: knobs, highlights and scrolling jump instead of sliding.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.still = on;
        self.states.set_reduce_motion(on);
    }

    /// Setting `i`'s value.
    pub fn value(&self, i: usize) -> Option<usize> {
        self.rows.get(i).map(|r| r.value)
    }

    /// Shows a value changed elsewhere (F3, F11, the window manager).
    pub fn set_value(&mut self, i: usize, value: usize) {
        if let Some(r) = self.rows.get_mut(i) {
            r.value = value.min(r.control.steps() - 1);
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.area.h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll_target = self.scroll_target.clamp(0.0, self.max_scroll());
    }

    /// The wheel turned `notches` (positive scrolls up).
    pub fn wheel(&mut self, notches: f32) {
        self.scroll_target -= notches * WHEEL_STEP;
        self.clamp_scroll();
    }

    /// Row `i` where it shows now (UI units).
    fn row_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.area.x,
            y: self.area.y + self.rows[i].y - self.scroll,
            w: self.area.w,
            h: ROW_H,
        }
    }

    /// Scrolls so that row `i` shows whole, its section's header too when it is first.
    fn reveal(&mut self, i: usize) {
        let Some(r) = self.rows.get(i) else { return };
        let top = if r.first { r.y - HEADER_H } else { r.y };
        let bottom = r.y + ROW_H;
        if top < self.scroll_target {
            self.scroll_target = top;
        } else if bottom > self.scroll_target + self.area.h {
            self.scroll_target = bottom - self.area.h;
        }
        self.clamp_scroll();
    }

    /// The control at `(x, y)` (UI units): a row, or Done (`len()`).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if self.done.rect.contains(x, y) {
            return Some(self.rows.len());
        }
        if !self.area.contains(x, y) {
            return None;
        }
        (0..self.rows.len()).find(|&i| self.row_rect(i).contains(x, y))
    }

    fn switch_rect(r: Rect) -> Rect {
        Rect {
            x: r.x + r.w - PAD - SWITCH_W,
            y: r.y + 0.5 * (r.h - SWITCH_H),
            w: SWITCH_W,
            h: SWITCH_H,
        }
    }

    fn track_rect(r: Rect) -> Rect {
        Rect {
            x: r.x + r.w - PAD - VALUE_W - 16.0 - TRACK_W,
            y: r.y + 0.5 * (r.h - TRACK_H),
            w: TRACK_W,
            h: TRACK_H,
        }
    }

    /// The segments of a choice in row `r`.
    fn segments(&self, r: Rect, labels: &[Text]) -> Vec<Rect> {
        let s = &self.scale;
        let w = labels
            .iter()
            .map(|t| t.width(s) + SEGMENT_PAD)
            .fold(SEGMENT_MIN, f32::max);
        let n = labels.len() as f32;
        let x0 = r.x + r.w - PAD - n * w;
        (0..labels.len())
            .map(|k| Rect {
                x: x0 + k as f32 * w,
                y: r.y + 0.5 * (r.h - SEGMENT_H),
                w,
                h: SEGMENT_H,
            })
            .collect()
    }

    /// Where row `i`'s control starts (UI units).
    fn control_left(&self, i: usize, r: Rect) -> f32 {
        match &self.rows[i].control {
            Control::Switch => Self::switch_rect(r).x,
            Control::Slider(_) => Self::track_rect(r).x - 0.5 * SLIDER_KNOB,
            Control::Choice(l) => self.segments(r, l).first().map_or(r.x + r.w, |s| s.x),
        }
    }

    /// The slider step nearest `x` on row `i`'s track.
    fn step_at(&self, i: usize, x: f32) -> usize {
        let n = self.rows[i].control.steps();
        let t = Self::track_rect(self.row_rect(i));
        let f = ((x - t.x) / t.w).clamp(0.0, 1.0);
        ((f * (n - 1) as f32).round() as usize).min(n - 1)
    }

    /// Sets row `i` to `value`: the event, if it changed.
    fn change(&mut self, i: usize, value: usize) -> Option<SettingsEvent> {
        let r = self.rows.get_mut(i)?;
        let value = value.min(r.control.steps() - 1);
        (r.value != value).then(|| {
            r.value = value;
            SettingsEvent::Changed { setting: i, value }
        })
    }

    /// The pointer moved to `(x, y)` (UI units), or left (`None`); a slider being
    /// dragged follows it.
    pub fn pointer(&mut self, at: Option<(f32, f32)>) -> Option<SettingsEvent> {
        self.pointer = at;
        self.states.hover(at.and_then(|(x, y)| self.hit(x, y)));
        let (i, (x, _)) = (self.dragging?, at?);
        let step = self.step_at(i, x);
        self.rows[i].shown.value = step as f32;
        self.change(i, step)
    }

    /// The left button went down at `(x, y)`: a slider's track grabs it.
    pub fn mouse_down(&mut self, x: f32, y: f32) -> Option<SettingsEvent> {
        let hit = self.hit(x, y);
        self.states.mouse_down(hit);
        let i = hit.filter(|&i| i < self.rows.len())?;
        if !matches!(self.rows[i].control, Control::Slider(_)) {
            return None;
        }
        let t = Self::track_rect(self.row_rect(i));
        if x < t.x - GRAB || x > t.x + t.w + GRAB {
            return None;
        }
        self.dragging = Some(i);
        let step = self.step_at(i, x);
        self.rows[i].shown.value = step as f32;
        self.change(i, step)
    }

    /// The left button went up at `(x, y)`: flips a switch, picks a segment, or Done.
    pub fn mouse_up(&mut self, x: f32, y: f32) -> Option<SettingsEvent> {
        let hit = self.hit(x, y);
        let released = self.states.mouse_up(hit);
        if self.dragging.take().is_some() {
            return None;
        }
        let i = released?;
        if i == self.rows.len() {
            return Some(SettingsEvent::Done);
        }
        let r = self.row_rect(i);
        match &self.rows[i].control {
            Control::Switch => {
                let v = 1 - self.rows[i].value.min(1);
                self.change(i, v)
            }
            Control::Choice(l) => {
                let k = self.segments(r, l).iter().position(|s| s.contains(x, y))?;
                self.change(i, k)
            }
            Control::Slider(_) => None,
        }
    }

    /// A key went down or up: moves focus, or acts on the focused control.
    pub fn key(&mut self, key: Key, down: bool) -> Option<SettingsEvent> {
        let i = self.states.key(key, down);
        if let Some(f) = self.states.focused().filter(|_| key != Key::Activate) {
            self.reveal(f);
        }
        let i = i?;
        if i == self.rows.len() {
            return Some(SettingsEvent::Done);
        }
        let row = &self.rows[i];
        match row.control {
            Control::Switch => self.change(i, 1 - row.value.min(1)),
            Control::Choice(_) => self.change(i, (row.value + 1) % row.control.steps()),
            Control::Slider(_) => None,
        }
    }

    /// Left (-1) or Right (+1) on the focused setting.
    pub fn adjust(&mut self, delta: isize) -> Option<SettingsEvent> {
        let i = self.states.focused().filter(|&i| i < self.rows.len())?;
        self.states.set_focus(Some(i), true);
        self.reveal(i);
        let row = &self.rows[i];
        let n = row.control.steps() as isize;
        let next = (row.value as isize + delta).clamp(0, n - 1) as usize;
        self.change(i, next)
    }

    /// Focuses setting `i` with its ring showing (or nothing), scrolling it into view.
    pub fn focus(&mut self, i: Option<usize>) {
        self.states.set_focus(i, true);
        if let Some(i) = i {
            self.reveal(i);
        }
    }

    /// Whether a slider is being dragged.
    pub fn dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Drops a press or a drag (the window lost focus).
    pub fn cancel(&mut self) {
        self.states.cancel();
        self.dragging = None;
    }

    /// Advances scrolling and the controls' motion by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        let k = 1.0 - (-dt / SCROLL_TAU).exp();
        self.scroll += (self.scroll_target - self.scroll) * k;
        if self.still || (self.scroll - self.scroll_target).abs() < 0.05 {
            self.scroll = self.scroll_target;
        }
        for (i, r) in self.rows.iter_mut().enumerate() {
            let target = r.value as f32;
            if self.still || self.dragging == Some(i) {
                r.shown.value = target;
            } else {
                r.shown.step(target, dt, SLIDE_TAU, SLIDE_TAU);
                if (r.shown.value - target).abs() < 0.01 {
                    r.shown.value = target;
                }
            }
        }
        self.states.update(dt);
    }

    /// The clip for the cards, fading an edge while there is more beyond it.
    fn clip(&self) -> Clip {
        let s = &self.scale;
        Clip {
            rect: px_rect(s, self.area.outset(8.0)),
            fade_top: self.scroll.min(FADE) * s.px,
            fade_bottom: (self.max_scroll() - self.scroll).clamp(0.0, FADE) * s.px,
            opacity: 1.0,
        }
    }

    /// Adds the screen's shapes and text.
    pub fn draw(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: Option<&GlyphAtlas>) {
        let s = &self.scale;
        if let Some(atlas) = atlas {
            let title = Rect {
                x: 0.0,
                y: 36.0,
                w: s.width,
                h: 40.0,
            };
            self.title.centred(text, atlas, s, title, WHITE_TEXT);
        }
        let clip = self.clip();
        shapes.set_clip(clip);
        text.set_clip(clip);
        let top = self.area.y - self.scroll;
        let visible =
            |y: f32, h: f32| y + h >= self.area.y - 8.0 && y <= self.area.y + self.area.h + 8.0;
        // Headers, and one glass card under each section's rows.
        let mut start = 0;
        for h in &self.headers {
            let y = top + h.y;
            if let Some(atlas) = atlas
                && visible(y, HEADER_H)
            {
                h.text.draw_at(
                    text,
                    atlas,
                    s,
                    self.area.x + 6.0,
                    y + HEADER_H - 12.0,
                    FAINT,
                );
            }
            let end = self.rows[start..]
                .iter()
                .position(|r| r.last)
                .map_or(self.rows.len(), |k| start + k + 1);
            if end > start {
                let card = Rect {
                    x: self.area.x,
                    y: top + self.rows[start].y,
                    w: self.area.w,
                    h: (end - start) as f32 * ROW_H,
                };
                if visible(card.y, card.h) {
                    shapes.glass_rect(s, card, CARD_RADIUS, CARD_TINT, 0.55);
                }
            }
            start = end;
        }
        for i in 0..self.rows.len() {
            let r = self.row_rect(i);
            if visible(r.y, r.h) {
                self.draw_row(shapes, text, atlas, i, r, clip);
            }
        }
        shapes.set_clip(Clip::NONE);
        text.set_clip(Clip::NONE);
        let n = self.rows.len();
        draw_button(
            shapes,
            text,
            atlas,
            s,
            self.done.rect,
            &self.done_label,
            self.states.look(n),
            self.states.enabled(n),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        i: usize,
        r: Rect,
        clip: Clip,
    ) {
        let s = &self.scale;
        let row = &self.rows[i];
        let look = self.states.look(i);
        let inner = r.outset(-4.0);
        if look.hover > 0.0 {
            let mut c = ROW_HOVER;
            c[3] *= look.hover;
            shapes.rounded(s, inner, CARD_RADIUS - 4.0, c);
        }
        if !row.last {
            let line = Rect {
                x: r.x + PAD,
                y: r.y + r.h - 0.5,
                w: r.w - 2.0 * PAD,
                h: 1.0,
            };
            shapes.rounded(s, line, 0.0, HAIRLINE);
        }
        draw_focus_ring(shapes, s, inner, CARD_RADIUS - 4.0, look.focus);
        let shown = row.shown.value;
        match &row.control {
            Control::Switch => {
                let sw = Self::switch_rect(r);
                shapes.rounded(s, sw, 0.5 * sw.h, mix(TRACK_OFF, ACCENT, shown));
                let d = sw.h - 2.0 * KNOB_INSET;
                let knob = Rect {
                    x: sw.x + KNOB_INSET + shown * (sw.w - 2.0 * KNOB_INSET - d),
                    y: sw.y + KNOB_INSET,
                    w: d,
                    h: d,
                };
                shapes.rounded(s, knob, 0.5 * d, KNOB);
            }
            Control::Slider(labels) => {
                let t = Self::track_rect(r);
                let f = shown / (labels.len().max(2) - 1) as f32;
                shapes.rounded(s, t, 0.5 * t.h, TRACK_OFF);
                shapes.rounded(s, Rect { w: t.w * f, ..t }, 0.5 * t.h, ACCENT);
                let d = SLIDER_KNOB
                    * if look.still {
                        1.0
                    } else {
                        1.0 + 0.1 * look.press
                    };
                let knob = Rect {
                    x: t.x + t.w * f - 0.5 * d,
                    y: t.y + 0.5 * t.h - 0.5 * d,
                    w: d,
                    h: d,
                };
                shapes.rounded(s, knob, 0.5 * d, KNOB);
                if let Some(atlas) = atlas
                    && let Some(v) = labels.get(row.value)
                {
                    let right = r.x + r.w - PAD;
                    let line = Rect {
                        x: right - v.width(s),
                        ..r
                    };
                    v.left_in(text, atlas, s, line, SOFT);
                }
            }
            Control::Choice(labels) => {
                let segs = self.segments(r, labels);
                if let (Some(a), Some(b)) = (segs.first(), segs.last()) {
                    let all = Rect {
                        w: b.x + b.w - a.x,
                        ..*a
                    };
                    shapes.rounded(s, all, 0.5 * all.h, SEGMENTS);
                    let hovered = self
                        .pointer
                        .filter(|_| look.hover > 0.0)
                        .and_then(|(x, y)| segs.iter().position(|g| g.contains(x, y)));
                    if let Some(h) = hovered.filter(|&h| h != row.value) {
                        let mut c = SEGMENT_HOVER;
                        c[3] *= look.hover;
                        let g = segs[h].outset(-3.0);
                        shapes.rounded(s, g, 0.5 * g.h, c);
                    }
                    let on = Rect {
                        x: a.x + shown * a.w,
                        ..*a
                    }
                    .outset(-3.0);
                    shapes.rounded(s, on, 0.5 * on.h, SEGMENT_ON);
                    if let Some(atlas) = atlas {
                        for (k, (g, t)) in segs.iter().zip(labels).enumerate() {
                            let c = if k == row.value { WHITE_TEXT } else { SOFT };
                            t.centred(text, atlas, s, *g, c);
                        }
                    }
                }
            }
        }
        let Some(atlas) = atlas else { return };
        // Text stops short of the control.
        let right = self.control_left(i, r) - CONTROL_GAP;
        text.set_clip(Clip {
            rect: [clip.rect[0], clip.rect[1], right * s.px, clip.rect[3]],
            ..clip
        });
        let x = r.x + PAD;
        row.label.draw_at(text, atlas, s, x, r.y + 30.0, WHITE_TEXT);
        row.detail.draw_at(text, atlas, s, x, r.y + 51.0, SOFT);
        text.set_clip(clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modern::WEIGHTS;

    fn sections() -> Vec<SectionSpec> {
        let setting = |label: &str, control, value| SettingSpec {
            label: label.into(),
            detail: "What it does".into(),
            control,
            value,
        };
        let steps = |n: usize| (0..n).map(|i| format!("{i}")).collect::<Vec<_>>();
        vec![
            SectionSpec {
                title: "Display".into(),
                settings: vec![
                    setting("VSync", ControlSpec::Switch, 1),
                    setting("Max Frame Rate", ControlSpec::Slider(steps(26)), 25),
                ],
            },
            SectionSpec {
                title: "Interface".into(),
                settings: vec![
                    setting("Menu Size", ControlSpec::Choice(steps(4)), 1),
                    setting("Reduce Motion", ControlSpec::Switch, 0),
                ],
            },
        ]
    }

    fn screen(w: u32, h: u32) -> SettingsScreen {
        let fonts = Fonts::new(&WEIGHTS);
        SettingsScreen::new(UiScale::new(w, h), &fonts, sections())
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.x + 0.5 * r.w, r.y + 0.5 * r.h)
    }

    #[test]
    fn clicks_flip_switches_pick_segments_and_drag_sliders() {
        let mut sc = screen(1600, 900);
        assert_eq!(sc.len(), 4);
        // A click anywhere on a switch's row flips it.
        let (x, y) = centre(sc.row_rect(0));
        assert_eq!(sc.mouse_down(x, y), None);
        assert_eq!(
            sc.mouse_up(x, y),
            Some(SettingsEvent::Changed {
                setting: 0,
                value: 0
            })
        );
        // Released off the row it went down on: nothing.
        sc.mouse_down(x, y);
        let (_, y3) = centre(sc.row_rect(3));
        assert_eq!(sc.mouse_up(x, y3), None);
        // The third segment.
        let r = sc.row_rect(2);
        let Control::Choice(l) = &sc.rows[2].control else {
            panic!()
        };
        let (sx, sy) = centre(sc.segments(r, l)[2]);
        sc.mouse_down(sx, sy);
        assert_eq!(
            sc.mouse_up(sx, sy),
            Some(SettingsEvent::Changed {
                setting: 2,
                value: 2
            })
        );
        // The slider follows the pointer from its track; the left end is step 0.
        let t = SettingsScreen::track_rect(sc.row_rect(1));
        let ty = t.y + 1.0;
        assert_eq!(
            sc.mouse_down(t.x + t.w, ty),
            None,
            "already at the last step"
        );
        assert_eq!(
            sc.pointer(Some((t.x - 40.0, ty + 30.0))),
            Some(SettingsEvent::Changed {
                setting: 1,
                value: 0
            })
        );
        assert_eq!(
            sc.pointer(Some((t.x + 0.5 * t.w, ty))),
            Some(SettingsEvent::Changed {
                setting: 1,
                value: 13
            })
        );
        assert_eq!(sc.mouse_up(t.x + 0.5 * t.w, ty), None);
        assert_eq!(sc.pointer(Some((t.x, ty))), None, "the drag ended");
        // Done.
        let (dx, dy) = centre(sc.done.rect);
        sc.mouse_down(dx, dy);
        assert_eq!(sc.mouse_up(dx, dy), Some(SettingsEvent::Done));
    }

    #[test]
    fn keyboard_moves_focus_and_changes_values() {
        let mut sc = screen(1600, 900);
        assert_eq!(sc.adjust(1), None, "nothing focused");
        sc.key(Key::Next, true);
        // Enter flips the switch on release.
        assert_eq!(sc.key(Key::Activate, true), None);
        assert_eq!(
            sc.key(Key::Activate, false),
            Some(SettingsEvent::Changed {
                setting: 0,
                value: 0
            })
        );
        assert_eq!(
            sc.adjust(1),
            Some(SettingsEvent::Changed {
                setting: 0,
                value: 1
            })
        );
        assert_eq!(sc.adjust(1), None, "already on");
        sc.key(Key::Next, true);
        assert_eq!(
            sc.adjust(-1),
            Some(SettingsEvent::Changed {
                setting: 1,
                value: 24
            })
        );
        // A choice steps round with Enter.
        sc.key(Key::Next, true);
        sc.set_value(2, 3);
        sc.key(Key::Activate, true);
        assert_eq!(
            sc.key(Key::Activate, false),
            Some(SettingsEvent::Changed {
                setting: 2,
                value: 0
            })
        );
        // Past the last setting: Done.
        sc.key(Key::Next, true);
        sc.key(Key::Next, true);
        sc.key(Key::Activate, true);
        assert_eq!(sc.key(Key::Activate, false), Some(SettingsEvent::Done));
    }

    #[test]
    fn short_windows_scroll_to_the_focused_setting() {
        let mut sc = screen(640, 360);
        assert!(sc.max_scroll() > 0.0);
        for _ in 0..4 {
            sc.key(Key::Next, true);
        }
        sc.set_reduce_motion(true);
        sc.update(0.016);
        let r = sc.row_rect(3);
        assert!(r.y + r.h <= sc.area.y + sc.area.h + 0.01);
        assert!(sc.area.y + sc.area.h < sc.done.rect.y);
        // Back to the top shows the first header too.
        sc.key(Key::Next, true);
        sc.key(Key::Next, true);
        sc.update(0.016);
        assert_eq!(sc.scroll, 0.0);
        // Text never runs under a control.
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        sc.draw(&mut shapes, &mut text, None);
        assert!(shapes.shapes().iter().filter(|s| s.glass).count() >= 2);
    }

    #[test]
    fn values_survive_a_new_layout_and_knobs_slide() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut sc = screen(1600, 900);
        let (x, y) = centre(sc.row_rect(3));
        sc.mouse_down(x, y);
        sc.mouse_up(x, y);
        sc.update(0.016);
        let mid = sc.rows[3].shown.value;
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        sc.layout(UiScale::new(1280, 720), &fonts);
        assert_eq!(sc.value(3), Some(1));
        for _ in 0..60 {
            sc.update(0.016);
        }
        assert_eq!(sc.rows[3].shown.value, 1.0);
        // Reduced motion: straight there.
        sc.set_reduce_motion(true);
        sc.set_value(3, 0);
        sc.update(0.016);
        assert_eq!(sc.rows[3].shown.value, 0.0);
    }
}
