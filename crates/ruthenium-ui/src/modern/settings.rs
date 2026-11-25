//! The Settings screen: settings in grouped glass cards, one row each with a label, a
//! line saying what it does, and a control on the right: a switch, a slider over fixed
//! steps, or a segmented choice. Done at the bottom.
//!
//! Rows take focus like buttons (Up/Down, Tab); Left/Right change the focused setting,
//! Enter/Space flip a switch or step a choice. A click flips a switch anywhere on its
//! row and picks the segment under the pointer; a slider follows the pointer from the
//! moment it goes down on its track.
//!
//! With many sections the screen has one page for each, and a carousel of their names at
//! the top: at most five whole ones show, the next ones fade away beyond them. The wheel
//! over it scrolls it, a click (or Left/Right with it focused) turns to a section; its
//! highlight glides there and the pages fade into each other.
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
/// The carousel: where it is, its buttons, how much of the next ones shows on either side,
/// and how many whole ones show at most.
const TITLE_Y: f32 = 26.0;
const TABS_Y: f32 = 78.0;
const TAB_W: f32 = 128.0;
const TAB_H: f32 = 42.0;
const TAB_GAP: f32 = 10.0;
const TAB_PITCH: f32 = TAB_W + TAB_GAP;
const TAB_PEEK: f32 = 64.0;
const TABS_SHOWN: usize = 5;
/// Room between the carousel and the cards.
const TABS_SPACE: f32 = 24.0;
const TAB_SCROLL_TAU: f32 = 0.09;
const TAB_GLIDE_TAU: f32 = 0.065;
/// Seconds a page takes to come and to go, and how far the last has to have gone before
/// the next comes.
const PAGE_IN: f32 = 0.16;
const PAGE_OUT: f32 = 0.09;
const PAGE_AFTER: f32 = 0.4;
/// What a section without settings says.
const NOTHING: &str = "Nothing here yet";

/// Time constant of a switch's knob, a choice's highlight and a slider stepped from the
/// keyboard.
const SLIDE_TAU: f32 = 0.08;

const CARD_TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.035];
const ROW_HOVER: [f32; 4] = [0.92, 0.94, 1.0, 0.05];
const TAB_TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.04];
const TAB_HOVER: [f32; 4] = [0.90, 0.93, 1.0, 0.12];
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
    /// Its page, and its top in the page's content (UI units, before scrolling).
    page: usize,
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

/// Rows that show together: one section's, or all of them.
#[derive(Debug, Clone, Copy, Default)]
struct Page {
    content_h: f32,
    scroll: f32,
    scroll_target: f32,
    /// How far it has come, from 0 to 1.
    shown: f32,
}

/// The carousel of the sections' names.
#[derive(Debug, Clone)]
struct Tabs {
    labels: Vec<Text>,
    states: ButtonStates,
    /// Where it shows, the fading ends included (UI units), and the whole buttons in it.
    view: Rect,
    whole: usize,
    /// How far it is scrolled, in UI units, and is to be: what the wheel asked for, and
    /// the button nearest to that.
    scroll: f32,
    wish: f32,
    /// Where the highlight is, in buttons.
    glide: f32,
}

impl Tabs {
    /// How far it can scroll.
    fn max(&self) -> f32 {
        self.labels.len().saturating_sub(self.whole) as f32 * TAB_PITCH
    }

    fn target(&self) -> f32 {
        (self.wish / TAB_PITCH).round() * TAB_PITCH
    }

    /// Scrolls so that button `i` is in the middle, as far as there are others.
    fn reveal(&mut self, i: usize) {
        let first = i as f32 - 0.5 * (self.whole as f32 - 1.0);
        self.wish = (first * TAB_PITCH).clamp(0.0, self.max());
    }

    /// Button `i`, or the place `i` buttons along, as it shows now (UI units).
    fn rect(&self, i: f32) -> Rect {
        Rect {
            x: self.view.x + TAB_PEEK + i * TAB_PITCH - self.scroll,
            y: self.view.y,
            w: TAB_W,
            h: TAB_H,
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.view.contains(x, y) {
            return None;
        }
        (0..self.labels.len()).find(|&i| self.rect(i as f32).contains(x, y))
    }

    fn update(&mut self, dt: f32, selected: usize, still: bool) {
        let (scroll, glide) = (self.target(), selected as f32);
        self.scroll += (scroll - self.scroll) * (1.0 - (-dt / TAB_SCROLL_TAU).exp());
        self.glide += (glide - self.glide) * (1.0 - (-dt / TAB_GLIDE_TAU).exp());
        if still || (self.scroll - scroll).abs() < 0.05 {
            self.scroll = scroll;
        }
        if still || (self.glide - glide).abs() < 0.001 {
            self.glide = glide;
        }
        self.states.update(dt);
    }
}

/// Slow at both ends.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    /// What the screen is called.
    name: String,
    title: Text,
    headers: Vec<Header>,
    rows: Vec<Row>,
    /// Where the cards show (UI units).
    area: Rect,
    /// One page for all sections, or one each with the carousel; and the one that shows.
    pages: Vec<Page>,
    page: usize,
    tabs: Option<Tabs>,
    nothing: Text,
    /// The slider being dragged.
    dragging: Option<usize>,
    pointer: Option<(f32, f32)>,
    still: bool,
}

impl SettingsScreen {
    /// A screen called `name` with these settings.
    pub fn new(
        scale: UiScale,
        fonts: &Fonts,
        name: &str,
        sections: Vec<SectionSpec>,
    ) -> SettingsScreen {
        SettingsScreen::with(scale, fonts, name, sections, false)
    }

    /// The same with a page for each section, and the carousel of their names.
    pub fn paged(
        scale: UiScale,
        fonts: &Fonts,
        name: &str,
        sections: Vec<SectionSpec>,
    ) -> SettingsScreen {
        SettingsScreen::with(scale, fonts, name, sections, true)
    }

    fn with(
        scale: UiScale,
        fonts: &Fonts,
        name: &str,
        sections: Vec<SectionSpec>,
        paged: bool,
    ) -> SettingsScreen {
        let n: usize = sections.iter().map(|s| s.settings.len()).sum();
        let pages = if paged { sections.len().max(1) } else { 1 };
        let blank = || Text::new(fonts, "", &scale, 12.0, Weight::REGULAR, 0.0);
        let mut screen = SettingsScreen {
            scale,
            // The rows, Done, and the carousel.
            states: ButtonStates::new(n + 1 + usize::from(paged)),
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
            name: name.to_owned(),
            title: blank(),
            headers: Vec::new(),
            rows: Vec::new(),
            area: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            pages: vec![Page::default(); pages],
            page: 0,
            tabs: paged.then(|| Tabs {
                labels: Vec::new(),
                states: ButtonStates::new(pages),
                view: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                },
                whole: 1,
                scroll: 0.0,
                wish: 0.0,
                glide: 0.0,
            }),
            nothing: blank(),
            dragging: None,
            pointer: None,
            still: false,
            sections,
        };
        screen.pages[0].shown = 1.0;
        screen.layout(scale, fonts);
        screen.enable_rows();
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
        let size = if self.tabs.is_some() { 24.0 } else { 28.0 };
        self.title = Text::new(fonts, &self.name, s, size, Weight::SEMIBOLD, 0.01);
        self.nothing = Text::new(fonts, NOTHING, s, 16.0, Weight::MEDIUM, 0.0);
        let mut top = TOP;
        if let Some(tabs) = &mut self.tabs {
            top = TABS_Y + TAB_H + TABS_SPACE;
            tabs.labels = self
                .sections
                .iter()
                .map(|t| Text::new(fonts, &t.title, s, 15.5, Weight::SEMIBOLD, 0.01))
                .collect();
            let room = scale.width - 2.0 * TAB_PEEK + TAB_GAP;
            tabs.whole = ((room / TAB_PITCH) as usize)
                .clamp(1, TABS_SHOWN)
                .min(tabs.labels.len().max(1));
            let view_w = tabs.whole as f32 * TAB_PITCH - TAB_GAP + 2.0 * TAB_PEEK;
            tabs.view = Rect {
                x: cx - 0.5 * view_w,
                y: TABS_Y,
                w: view_w,
                h: TAB_H,
            };
            tabs.reveal(self.page);
            tabs.scroll = tabs.target();
        }
        self.area = Rect {
            x: cx - 0.5 * w,
            y: top,
            w,
            h: (y0 - FOOTER_GAP - top).max(ROW_H),
        };
        let old = std::mem::take(&mut self.rows);
        self.headers.clear();
        let mut y = 0.0;
        for (k, section) in self.sections.iter().enumerate() {
            let page = if self.tabs.is_some() { k } else { 0 };
            if self.tabs.is_some() {
                y = 0.0;
            } else if k > 0 {
                y += SECTION_GAP;
            }
            if self.tabs.is_none() {
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
            }
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
                    page,
                    y,
                    first: j == 0,
                    last: j + 1 == count,
                });
                y += ROW_H;
            }
            self.pages[page].content_h = y;
        }
        for page in 0..self.pages.len() {
            self.clamp_scroll(page);
            self.pages[page].scroll = self.pages[page].scroll_target;
        }
    }

    /// Only the rows of the page that shows take the pointer and the focus.
    fn enable_rows(&mut self) {
        for i in 0..self.rows.len() {
            self.states.set_enabled(i, self.rows[i].page == self.page);
        }
    }

    /// The page that shows.
    pub fn page(&self) -> usize {
        self.page
    }

    /// Turns to page `to`.
    pub fn turn_to(&mut self, to: usize) {
        if to >= self.pages.len() {
            return;
        }
        if let Some(tabs) = &mut self.tabs {
            tabs.reveal(to);
        }
        if to == self.page {
            return;
        }
        self.page = to;
        self.dragging = None;
        // What had the focus on the page before has it no more.
        if self.states.focused().is_some_and(|f| f < self.rows.len()) {
            self.states.set_focus(None, false);
        }
        self.states.hover(None);
        self.enable_rows();
    }

    /// Turns `by` pages on, as far as there are any.
    pub fn turn(&mut self, by: isize) {
        let last = self.pages.len() as isize - 1;
        self.turn_to((self.page as isize + by).clamp(0, last) as usize);
    }

    /// The control number of the carousel.
    fn carousel(&self) -> usize {
        self.rows.len() + 1
    }

    /// Reduced motion: knobs, highlights and scrolling jump instead of sliding.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.still = on;
        self.states.set_reduce_motion(on);
        if let Some(tabs) = &mut self.tabs {
            tabs.states.set_reduce_motion(on);
        }
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

    fn max_scroll(&self, page: usize) -> f32 {
        (self.pages[page].content_h - self.area.h).max(0.0)
    }

    fn clamp_scroll(&mut self, page: usize) {
        let max = self.max_scroll(page);
        let target = &mut self.pages[page].scroll_target;
        *target = target.clamp(0.0, max);
    }

    /// The wheel turned `notches` (positive scrolls up, or the carousel to the left).
    pub fn wheel(&mut self, notches: f32) {
        if let (Some(tabs), Some((x, y))) = (&mut self.tabs, self.pointer)
            && tabs.view.contains(x, y)
        {
            tabs.wish = (tabs.wish - notches * TAB_PITCH).clamp(0.0, tabs.max());
            return;
        }
        self.pages[self.page].scroll_target -= notches * WHEEL_STEP;
        self.clamp_scroll(self.page);
    }

    /// Row `i` where it shows now (UI units).
    fn row_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.area.x,
            y: self.area.y + self.rows[i].y - self.pages[self.rows[i].page].scroll,
            w: self.area.w,
            h: ROW_H,
        }
    }

    /// Scrolls so that row `i` shows whole, its section's header too when it is first.
    fn reveal(&mut self, i: usize) {
        let Some(r) = self.rows.get(i) else { return };
        let header = if r.first && self.tabs.is_none() {
            HEADER_H
        } else {
            0.0
        };
        let (page, top, bottom) = (r.page, r.y - header, r.y + ROW_H);
        let target = &mut self.pages[page].scroll_target;
        if top < *target {
            *target = top;
        } else if bottom > *target + self.area.h {
            *target = bottom - self.area.h;
        }
        self.clamp_scroll(page);
    }

    /// The control at `(x, y)` (UI units): a row, or Done (`len()`).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if self.done.rect.contains(x, y) {
            return Some(self.rows.len());
        }
        if !self.area.contains(x, y) {
            return None;
        }
        (0..self.rows.len())
            .find(|&i| self.rows[i].page == self.page && self.row_rect(i).contains(x, y))
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

    /// The names of the sections and settings whose words do not fit beside their controls
    /// or in their buttons.
    pub fn cut_off(&self) -> Vec<&str> {
        let s = &self.scale;
        let rows = self.rows.iter().enumerate().filter(|(i, row)| {
            let r = self.row_rect(*i);
            let room = self.control_left(*i, r) - CONTROL_GAP - (r.x + PAD);
            row.label.width(s).max(row.detail.width(s)) > room
        });
        let names = self.sections.iter().flat_map(|t| &t.settings);
        let names: Vec<&str> = names.map(|t| t.label.as_str()).collect();
        let tabs = self.tabs.iter().flat_map(|t| &t.labels).enumerate();
        let tabs = tabs.filter(|(_, label)| label.width(s) > TAB_W - 20.0);
        rows.map(|(i, _)| names[i])
            .chain(tabs.map(|(i, _)| self.sections[i].title.as_str()))
            .collect()
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
        if let Some(tabs) = &mut self.tabs {
            let over = at.and_then(|(x, y)| tabs.hit(x, y));
            tabs.states.hover(over);
        }
        let (i, (x, _)) = (self.dragging?, at?);
        let step = self.step_at(i, x);
        self.rows[i].shown.value = step as f32;
        self.change(i, step)
    }

    /// The left button went down at `(x, y)`: a slider's track grabs it.
    pub fn mouse_down(&mut self, x: f32, y: f32) -> Option<SettingsEvent> {
        let hit = self.hit(x, y);
        self.states.mouse_down(hit);
        if let Some(tabs) = &mut self.tabs {
            let over = tabs.hit(x, y);
            tabs.states.mouse_down(over);
        }
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
        let turned = self.tabs.as_mut().and_then(|tabs| {
            let over = tabs.hit(x, y);
            tabs.states.mouse_up(over)
        });
        if let Some(to) = turned {
            self.turn_to(to);
            return None;
        }
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
        let row = self.rows.get(i)?;
        match row.control {
            Control::Switch => self.change(i, 1 - row.value.min(1)),
            Control::Choice(_) => self.change(i, (row.value + 1) % row.control.steps()),
            Control::Slider(_) => None,
        }
    }

    /// Left (-1) or Right (+1) on the focused setting.
    pub fn adjust(&mut self, delta: isize) -> Option<SettingsEvent> {
        if self.tabs.is_some() && self.states.focused() == Some(self.carousel()) {
            self.states.set_focus(Some(self.carousel()), true);
            self.turn(delta);
            return None;
        }
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
        if let Some(page) = i.and_then(|i| self.rows.get(i)).map(|r| r.page) {
            self.turn_to(page);
        }
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
        if let Some(tabs) = &mut self.tabs {
            tabs.states.cancel();
        }
        self.dragging = None;
    }

    /// Advances scrolling and the controls' motion by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        let k = 1.0 - (-dt / SCROLL_TAU).exp();
        for p in &mut self.pages {
            p.scroll += (p.scroll_target - p.scroll) * k;
            if self.still || (p.scroll - p.scroll_target).abs() < 0.05 {
                p.scroll = p.scroll_target;
            }
        }
        // The page that showed goes first, and the next comes once it has nearly gone.
        let mut left = 0.0f32;
        for (i, p) in self.pages.iter_mut().enumerate() {
            if i != self.page {
                p.shown = (p.shown - dt / PAGE_OUT).max(0.0);
                left = left.max(p.shown);
            }
        }
        if left < PAGE_AFTER {
            let p = &mut self.pages[self.page];
            p.shown = (p.shown + dt / PAGE_IN).min(1.0);
        }
        if let Some(tabs) = &mut self.tabs {
            tabs.update(dt, self.page, self.still);
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

    /// The clip for a page's cards, fading an edge while there is more beyond it.
    fn clip(&self, page: usize) -> Clip {
        let s = &self.scale;
        let p = &self.pages[page];
        Clip {
            rect: px_rect(s, self.area.outset(8.0)),
            fade_top: p.scroll.min(FADE) * s.px,
            fade_bottom: (self.max_scroll(page) - p.scroll).clamp(0.0, FADE) * s.px,
            opacity: smooth(p.shown),
            ..Clip::NONE
        }
    }

    /// Draws the carousel: its buttons, the highlight on the one whose page shows, the
    /// names; both ends fade away.
    fn draw_tabs(
        &self,
        tabs: &Tabs,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
    ) {
        let s = &self.scale;
        // (The focus ring and the glass's edge reach a little over the buttons.)
        let mut clip = Clip::rect(px_rect(s, tabs.view));
        clip.rect[1] -= 12.0 * s.px;
        clip.rect[3] += 12.0 * s.px;
        (clip.fade_left, clip.fade_right) = (TAB_PEEK * s.px, TAB_PEEK * s.px);
        shapes.set_clip(clip);
        text.set_clip(clip);
        let seen = |r: &Rect| r.x + r.w > tabs.view.x && r.x < tabs.view.x + tabs.view.w;
        for i in 0..tabs.labels.len() {
            let r = tabs.rect(i as f32);
            if seen(&r) {
                let look = tabs.states.look(i);
                let r = r.scaled(look.size());
                shapes.glass_pill(s, r, mix(TAB_TINT, TAB_HOVER, look.hover), 0.6);
            }
        }
        let on = tabs.rect(tabs.glide).outset(-3.0);
        shapes.rounded(s, on, 0.5 * on.h, SEGMENT_ON);
        let focus = self.states.look(self.carousel()).focus;
        draw_focus_ring(shapes, s, on, 0.5 * on.h, focus);
        if let Some(atlas) = atlas {
            for (i, label) in tabs.labels.iter().enumerate() {
                let r = tabs.rect(i as f32);
                if seen(&r) {
                    // As the highlight comes, the name lights up.
                    let near = (1.0 - (tabs.glide - i as f32).abs()).max(0.0);
                    let lit = near.max(0.6 * tabs.states.look(i).hover);
                    label.centred(text, atlas, s, r, mix(SOFT, WHITE_TEXT, lit));
                }
            }
        }
        shapes.set_clip(Clip::NONE);
        text.set_clip(Clip::NONE);
    }

    /// Adds the screen's shapes and text.
    pub fn draw(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: Option<&GlyphAtlas>) {
        let s = &self.scale;
        if let Some(atlas) = atlas {
            let title = Rect {
                x: 0.0,
                y: if self.tabs.is_some() { TITLE_Y } else { 36.0 },
                w: s.width,
                h: 40.0,
            };
            self.title.centred(text, atlas, s, title, WHITE_TEXT);
        }
        if let Some(tabs) = &self.tabs {
            self.draw_tabs(tabs, shapes, text, atlas);
        }
        for page in 0..self.pages.len() {
            if self.pages[page].shown > 0.0 {
                self.draw_page(page, shapes, text, atlas);
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

    /// Headers, one glass card under each section's rows, and the rows.
    fn draw_page(
        &self,
        page: usize,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
    ) {
        let s = &self.scale;
        let clip = self.clip(page);
        shapes.set_clip(clip);
        text.set_clip(clip);
        let top = self.area.y - self.pages[page].scroll;
        let visible =
            |y: f32, h: f32| y + h >= self.area.y - 8.0 && y <= self.area.y + self.area.h + 8.0;
        if let Some(atlas) = atlas {
            for h in &self.headers {
                let y = top + h.y;
                if visible(y, HEADER_H) {
                    h.text.draw_at(
                        text,
                        atlas,
                        s,
                        self.area.x + 6.0,
                        y + HEADER_H - 12.0,
                        FAINT,
                    );
                }
            }
        }
        let mut first = None;
        for (i, row) in self.rows.iter().enumerate() {
            if row.page != page {
                continue;
            }
            let start = *first.get_or_insert(i);
            if row.last {
                first = None;
                let card = Rect {
                    x: self.area.x,
                    y: top + self.rows[start].y,
                    w: self.area.w,
                    h: (i + 1 - start) as f32 * ROW_H,
                };
                if visible(card.y, card.h) {
                    shapes.glass_rect(s, card, CARD_RADIUS, CARD_TINT, 0.55);
                }
            }
        }
        let mut empty = true;
        for i in 0..self.rows.len() {
            if self.rows[i].page != page {
                continue;
            }
            empty = false;
            let r = self.row_rect(i);
            if visible(r.y, r.h) {
                self.draw_row(shapes, text, atlas, i, r, clip);
            }
        }
        if let Some(atlas) = atlas.filter(|_| empty) {
            let line = Rect {
                h: ROW_H,
                ..self.area
            };
            self.nothing.centred(text, atlas, s, line, SOFT);
        }
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
        SettingsScreen::new(UiScale::new(w, h), &fonts, "Settings", sections())
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
        assert!(sc.max_scroll(0) > 0.0);
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
        assert_eq!(sc.pages[0].scroll, 0.0);
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

    /// Nine pages: the first has two settings, the second one, the others none.
    fn paged(w: u32, h: u32) -> SettingsScreen {
        let fonts = Fonts::new(&WEIGHTS);
        let mut all = sections();
        all[1].settings.truncate(1);
        for name in [
            "Online",
            "Resources",
            "Access",
            "Sound",
            "Chat",
            "Skin",
            "Lang",
        ] {
            all.push(SectionSpec {
                title: name.into(),
                settings: Vec::new(),
            });
        }
        SettingsScreen::paged(UiScale::new(w, h), &fonts, "MC Options", all)
    }

    fn settle(sc: &mut SettingsScreen) {
        for _ in 0..200 {
            sc.update(0.01);
        }
    }

    /// The buttons of the carousel that show whole, between its fading ends.
    fn whole(sc: &SettingsScreen) -> Vec<usize> {
        let t = sc.tabs.as_ref().unwrap();
        let (left, right) = (t.view.x + TAB_PEEK, t.view.x + t.view.w - TAB_PEEK);
        (0..t.labels.len())
            .filter(|&i| {
                let r = t.rect(i as f32);
                r.x >= left - 0.01 && r.x + r.w <= right + 0.01
            })
            .collect()
    }

    #[test]
    fn at_most_five_whole_buttons_show_and_the_chosen_one_is_in_the_middle() {
        let mut sc = paged(1600, 900);
        assert_eq!(whole(&sc), [0, 1, 2, 3, 4]);
        let view = sc.tabs.as_ref().unwrap().view;
        assert_eq!(view.x + 0.5 * view.w, 800.0);
        // The cards begin under it.
        assert!(sc.area.y > view.y + view.h);
        for (page, want) in [
            (2, [0, 1, 2, 3, 4]),
            (3, [1, 2, 3, 4, 5]),
            (6, [4, 5, 6, 7, 8]),
            (8, [4, 5, 6, 7, 8]),
        ] {
            sc.turn_to(page);
            settle(&mut sc);
            assert_eq!(whole(&sc), want, "{page}");
            assert_eq!(sc.tabs.as_ref().unwrap().glide, page as f32);
        }
        // A page there is not is not turned to, nor beyond the ends.
        sc.turn_to(9);
        assert_eq!(sc.page(), 8);
        sc.turn_to(7);
        sc.turn(3);
        assert_eq!(sc.page(), 8);
        sc.turn(-20);
        assert_eq!(sc.page(), 0);
        // A narrow window shows fewer, and few pages take no scrolling.
        assert!(whole(&paged(480, 480)).len() < 5);
        let fonts = Fonts::new(&WEIGHTS);
        let few = SettingsScreen::paged(UiScale::new(1600, 900), &fonts, "x", sections());
        let t = few.tabs.as_ref().unwrap();
        assert_eq!((t.whole, t.max()), (2, 0.0));
        assert_eq!(t.view.x + 0.5 * t.view.w, 800.0);
    }

    #[test]
    fn the_carousel_scrolls_smoothly_and_rests_on_whole_buttons() {
        let mut sc = paged(1600, 900);
        let view = sc.tabs.as_ref().unwrap().view;
        sc.pointer(Some(centre(view)));
        sc.wheel(-1.0);
        let scroll = |sc: &SettingsScreen| sc.tabs.as_ref().unwrap().scroll;
        let mut last = 0.0;
        for _ in 0..30 {
            sc.update(0.004);
            let now = scroll(&sc);
            assert!(now > last && now - last < 8.0, "{last} {now}");
            last = now;
        }
        settle(&mut sc);
        assert_eq!(scroll(&sc), TAB_PITCH);
        assert_eq!(whole(&sc), [1, 2, 3, 4, 5]);
        // Less than half a button's turn comes back, and it stops at the ends.
        sc.wheel(-0.4);
        settle(&mut sc);
        assert_eq!(scroll(&sc), TAB_PITCH);
        sc.wheel(-50.0);
        settle(&mut sc);
        assert_eq!(whole(&sc), [4, 5, 6, 7, 8]);
        sc.wheel(50.0);
        settle(&mut sc);
        assert_eq!(scroll(&sc), 0.0);
        // It did not turn the page, nor does the wheel off it scroll it.
        assert_eq!(sc.page(), 0);
        sc.pointer(Some(centre(sc.area)));
        sc.wheel(-1.0);
        settle(&mut sc);
        assert_eq!(scroll(&sc), 0.0);
    }

    #[test]
    fn a_click_on_a_name_turns_to_its_page() {
        let mut sc = paged(1600, 900);
        let button =
            |sc: &SettingsScreen, i: usize| centre(sc.tabs.as_ref().unwrap().rect(i as f32));
        // Row 2 is on the second page: not there yet.
        let (x, y) = centre(sc.row_rect(2));
        assert_eq!(sc.hit(x, y), Some(0));
        let (bx, by) = button(&sc, 1);
        assert_eq!(sc.mouse_down(bx, by), None);
        assert_eq!(sc.mouse_up(bx, by), None);
        assert_eq!(sc.page(), 1);
        assert_eq!(sc.hit(x, y), Some(2));
        // Released off it, nothing.
        let (ox, oy) = button(&sc, 3);
        sc.mouse_down(ox, oy);
        sc.mouse_up(bx, by);
        assert_eq!(sc.page(), 1);
        // What is under the fading ends and beyond is clicked only where it shows.
        let t = sc.tabs.as_ref().unwrap();
        assert_eq!(t.hit(t.view.x + t.view.w + 5.0, by), None);
        assert_eq!(t.hit(t.view.x + t.view.w - 1.0, by), Some(5));
    }

    #[test]
    fn pages_fade_into_each_other_and_the_highlight_glides() {
        let mut sc = paged(1600, 900);
        settle(&mut sc);
        sc.turn_to(1);
        sc.update(0.02);
        let shown = |sc: &SettingsScreen| (sc.pages[0].shown, sc.pages[1].shown);
        // The old one goes first.
        assert!(shown(&sc).0 < 1.0 && shown(&sc).0 > 0.0 && shown(&sc).1 == 0.0);
        let glide = sc.tabs.as_ref().unwrap().glide;
        assert!(glide > 0.0 && glide < 1.0, "{glide}");
        let mut last = (shown(&sc), glide);
        for _ in 0..80 {
            sc.update(0.004);
            let g = sc.tabs.as_ref().unwrap().glide;
            let now = (shown(&sc), g);
            assert!(now.0.0 <= last.0.0 && now.0.1 >= last.0.1 && now.1 >= last.1);
            assert!(now.0.1 - last.0.1 < 0.03 && last.0.0 - now.0.0 < 0.05);
            last = now;
        }
        assert_eq!(shown(&sc), (0.0, 1.0));
        // Turned back half way, both go on from where they are.
        sc.turn_to(0);
        sc.update(0.03);
        let before = shown(&sc);
        sc.turn_to(1);
        sc.update(0.0);
        assert_eq!(shown(&sc), before);
        // Faded pages are drawn fainter, and gone ones not at all.
        let drawn = |sc: &SettingsScreen| {
            let (mut shapes, mut text) = (ShapeList::new(), TextList::new());
            sc.draw(&mut shapes, &mut text, None);
            let cards = shapes.shapes().iter().filter(|s| s.glass);
            // (The carousel's own are whole.)
            let faint = cards.clone().filter(|s| s.clip.opacity < 1.0).count();
            let fading = cards.filter(|s| s.clip.fade_left > 0.0).count();
            (faint, fading)
        };
        assert_eq!(drawn(&sc).0, 1);
        settle(&mut sc);
        assert_eq!(drawn(&sc).0, 0);
        // Five and the one that fades; on the right there is one more only.
        assert_eq!(drawn(&sc).1, 6);
        // Reduced motion: no gliding.
        sc.set_reduce_motion(true);
        sc.turn_to(6);
        sc.update(0.001);
        let t = sc.tabs.as_ref().unwrap();
        assert_eq!((t.glide, t.scroll), (6.0, 4.0 * TAB_PITCH));
    }

    #[test]
    fn the_keyboard_turns_pages_and_stays_on_the_one_that_shows() {
        let mut sc = paged(1600, 900);
        // The two rows, Done, the carousel, and round.
        let mut order = Vec::new();
        for _ in 0..5 {
            sc.key(Key::Next, true);
            order.push(sc.states.focused().unwrap());
        }
        let (done, carousel) = (sc.len(), sc.len() + 1);
        assert_eq!(order, [0, 1, done, carousel, 0]);
        // Left and Right with the carousel focused.
        sc.key(Key::Previous, true);
        assert_eq!(sc.adjust(1), None);
        assert_eq!((sc.page(), sc.states.focused()), (1, Some(carousel)));
        sc.key(Key::Next, true);
        assert_eq!(sc.states.focused(), Some(2), "the second page's row");
        // Turning takes the focus from a row that is gone.
        sc.turn(1);
        assert_eq!(sc.states.focused(), None);
        sc.key(Key::Next, true);
        assert_eq!(sc.states.focused(), Some(done), "an empty page has Done");
        assert_eq!(sc.adjust(1), None);
        // Enter on the carousel does nothing.
        sc.key(Key::Next, true);
        sc.key(Key::Activate, true);
        assert_eq!(sc.key(Key::Activate, false), None);
        // Focusing a setting turns to its page.
        sc.focus(Some(0));
        assert_eq!(sc.page(), 0);
        // A new layout stays on the page.
        sc.turn_to(4);
        sc.layout(UiScale::new(1280, 720), &Fonts::new(&WEIGHTS));
        assert_eq!(sc.page(), 4);
        assert_eq!(whole(&sc), [2, 3, 4, 5, 6]);
    }
}
