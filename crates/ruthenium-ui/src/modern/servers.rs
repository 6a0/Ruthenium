//! The server list (Multiplayer): saved servers as glass rows with their icon, name,
//! message of the day, player count and latency, in a scrolling list; the buttons below.
//!
//! The screen keeps layout, shaped text, scrolling, selection and hover; the client keeps
//! the servers and their ping results and hands them over as [`RowView`]s whenever they
//! change.

use ruthenium_protocol::text::{Style, StyledText};

use super::{
    Action, Button, ButtonStates, Ease, FAINT, Rect, SOFT, ShapeList, Text, UiScale, WHITE_TEXT,
    draw_button, draw_focus_ring, mix, px_rect,
};
use crate::Clip;
use crate::text::{Fonts, GlyphAtlas, TextList, Weight};

/// How a server's ping is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ping {
    /// Waiting for the status or the pong.
    Pinging,
    /// Answered, in this many milliseconds.
    Latency(u64),
    /// Answered, but plays another version.
    Incompatible,
    Failed,
}

/// One server as the list shows it.
#[derive(Debug, Clone, Copy)]
pub struct RowView<'a> {
    pub name: &'a str,
    /// Message of the day (at most two lines show), or why the ping failed.
    pub motd: &'a StyledText,
    /// Right-hand line: the player count, or the server's version when incompatible.
    pub status: &'a StyledText,
    pub ping: Ping,
    /// The favicon's texture index, once uploaded.
    pub icon: Option<u32>,
}

/// Button indices, in tab order.
pub const JOIN: usize = 0;
pub const DIRECT: usize = 1;
pub const ADD: usize = 2;
pub const EDIT: usize = 3;
pub const DELETE: usize = 4;
pub const REFRESH: usize = 5;
pub const BACK: usize = 6;

const LIST_WIDTH: f32 = 760.0;
const ROW_H: f32 = 84.0;
const ROW_GAP: f32 = 10.0;
const ROW_RADIUS: f32 = 20.0;
const PAD: f32 = 12.0;
const ICON: f32 = 60.0;
const ICON_RADIUS: f32 = 14.0;
const TEXT_X: f32 = PAD + ICON + 16.0;
/// Room kept on the right of a row for the latency, bars and player count.
const RIGHT: f32 = 128.0;
const BUTTON_H: f32 = 44.0;
const BUTTON_GAP: f32 = 12.0;
const FOOTER_MARGIN: f32 = 30.0;
const FADE: f32 = 28.0;
/// Scrolling: units per wheel notch and the time constant of the glide.
const WHEEL_STEP: f32 = 64.0;
const SCROLL_TAU: f32 = 0.07;
/// Two clicks on one row this close together (seconds) join it (vanilla's 250 ms).
const DOUBLE_CLICK: f64 = 0.25;

const ROW_TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.03];
const ROW_HOVER: [f32; 4] = [0.90, 0.93, 1.0, 0.08];
const ROW_SELECTED: [f32; 4] = [0.80, 0.86, 1.0, 0.14];
const MOTD_DEFAULT: [f32; 4] = [0.82, 0.85, 0.95, 0.78];
const BAR_OFF: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
const BAR_GOOD: [f32; 4] = [0.45, 0.95, 0.62, 0.95];
const BAR_FAIR: [f32; 4] = [1.0, 0.84, 0.38, 0.95];
const BAR_POOR: [f32; 4] = [1.0, 0.48, 0.42, 0.95];
const BAR_FAILED: [f32; 4] = [1.0, 0.45, 0.45, 0.35];

/// A run of message text in one style.
#[derive(Debug, Clone)]
struct Run {
    text: Text,
    color: [f32; 4],
    underline: bool,
    strike: bool,
}

#[derive(Debug, Clone)]
struct Row {
    name: Text,
    initial: Text,
    motd: Vec<Vec<Run>>,
    status: Vec<Run>,
    latency: Option<Text>,
    ping: Ping,
    icon: Option<u32>,
    hover: Ease,
    selected: Ease,
}

/// A text colour for the dark glass: span colours as they are, dark ones lifted so they
/// stay readable.
fn span_color(style: &Style, default: [f32; 4]) -> [f32; 4] {
    let Some(rgb) = style.color else {
        return default;
    };
    let c = [
        ((rgb >> 16) & 0xFF) as f32 / 255.0,
        ((rgb >> 8) & 0xFF) as f32 / 255.0,
        (rgb & 0xFF) as f32 / 255.0,
        1.0,
    ];
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let lift = (0.35 - luma).max(0.0);
    mix(c, [1.0; 4], lift)
}

fn runs(fonts: &Fonts, scale: &UiScale, t: &StyledText, size: f32, default: [f32; 4]) -> Vec<Run> {
    t.spans
        .iter()
        .filter(|s| !s.text.is_empty())
        .map(|s| Run {
            text: Text::new(
                fonts,
                &s.text,
                scale,
                size,
                if s.style.bold {
                    Weight::EXTRABOLD
                } else {
                    Weight::REGULAR
                },
                0.0,
            ),
            color: span_color(&s.style, default),
            underline: s.style.underlined,
            strike: s.style.strikethrough,
        })
        .collect()
}

fn runs_width(runs: &[Run], scale: &UiScale) -> f32 {
    runs.iter().map(|r| r.text.width(scale)).sum()
}

/// Draws runs with the baseline's left end at `(x, y)` (UI units).
fn draw_runs(
    shapes: &mut ShapeList,
    text: &mut TextList,
    atlas: &GlyphAtlas,
    s: &UiScale,
    runs: &[Run],
    mut x: f32,
    y: f32,
) {
    for r in runs {
        let w = r.text.width(s);
        r.text.draw_at(text, atlas, s, x, y, r.color);
        let cap = r.text.cap / s.px;
        let thickness = (cap * 0.09).max(1.0);
        if r.underline {
            shapes.rounded(
                s,
                Rect {
                    x,
                    y: y + 1.5,
                    w,
                    h: thickness,
                },
                0.0,
                r.color,
            );
        }
        if r.strike {
            let mid = y - 0.45 * cap;
            shapes.rounded(
                s,
                Rect {
                    x,
                    y: mid,
                    w,
                    h: thickness,
                },
                0.0,
                r.color,
            );
        }
        x += w;
    }
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct ServerListScreen {
    pub scale: UiScale,
    pub buttons: Vec<Button>,
    pub states: ButtonStates,
    /// The list's area (UI units).
    pub list: Rect,
    labels: Vec<Text>,
    title: Text,
    empty: [Text; 2],
    rows: Vec<Row>,
    selected: Option<usize>,
    hovered: Option<usize>,
    scroll: f32,
    scroll_target: f32,
    /// The last click on a row: which and when (seconds).
    last_click: Option<(usize, f64)>,
}

impl ServerListScreen {
    pub fn new(scale: UiScale, fonts: &Fonts) -> ServerListScreen {
        let entries = [
            ("Join Server", Action::Join),
            ("Direct Connect", Action::DirectConnect),
            ("Add Server", Action::AddServer),
            ("Edit", Action::EditServer),
            ("Delete", Action::DeleteServer),
            ("Refresh", Action::Refresh),
            ("Back", Action::Back),
        ];
        let mut screen = ServerListScreen {
            scale,
            buttons: entries
                .iter()
                .map(|&(label, action)| Button {
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    },
                    label,
                    action,
                })
                .collect(),
            states: ButtonStates::new(entries.len()),
            list: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            labels: Vec::new(),
            title: Text::new(fonts, "Multiplayer", &scale, 28.0, Weight::SEMIBOLD, 0.01),
            empty: [
                Text::new(fonts, "No servers yet", &scale, 20.0, Weight::SEMIBOLD, 0.0),
                Text::new(
                    fonts,
                    "Add one, or connect directly.",
                    &scale,
                    15.0,
                    Weight::REGULAR,
                    0.0,
                ),
            ],
            rows: Vec::new(),
            selected: None,
            hovered: None,
            scroll: 0.0,
            scroll_target: 0.0,
            last_click: None,
        };
        screen.layout(scale, fonts);
        screen
    }

    /// Lays the screen out for `scale`, keeping rows, selection and scrolling; their text
    /// is shaped again by the next [`Self::set_rows`].
    pub fn layout(&mut self, scale: UiScale, fonts: &Fonts) {
        self.scale = scale;
        let w = LIST_WIDTH.min(scale.width - 64.0).max(320.0);
        let cx = 0.5 * scale.width;
        let footer = 2.0 * BUTTON_H + BUTTON_GAP;
        let bottom = scale.height - FOOTER_MARGIN - footer - 26.0;
        self.list = Rect {
            x: cx - 0.5 * w,
            y: 104.0,
            w,
            h: (bottom - 104.0).max(ROW_H),
        };
        // Top row: three buttons; bottom row: four, both as wide as the list allows.
        let row_w = w.min(690.0);
        let top_w = (row_w - 2.0 * BUTTON_GAP) / 3.0;
        let bottom_w = (row_w - 3.0 * BUTTON_GAP) / 4.0;
        let y0 = scale.height - FOOTER_MARGIN - footer;
        let x0 = cx - 0.5 * row_w;
        for (i, b) in self.buttons.iter_mut().enumerate() {
            b.rect = if i < 3 {
                Rect {
                    x: x0 + i as f32 * (top_w + BUTTON_GAP),
                    y: y0,
                    w: top_w,
                    h: BUTTON_H,
                }
            } else {
                let j = (i - 3) as f32;
                Rect {
                    x: x0 + j * (bottom_w + BUTTON_GAP),
                    y: y0 + BUTTON_H + BUTTON_GAP,
                    w: bottom_w,
                    h: BUTTON_H,
                }
            };
        }
        self.labels = self
            .buttons
            .iter()
            .map(|b| Text::new(fonts, b.label, &scale, 16.0, Weight::SEMIBOLD, 0.01))
            .collect();
        self.title = Text::new(fonts, "Multiplayer", &scale, 28.0, Weight::SEMIBOLD, 0.01);
        self.empty = [
            Text::new(fonts, "No servers yet", &scale, 20.0, Weight::SEMIBOLD, 0.0),
            Text::new(
                fonts,
                "Add one, or connect directly.",
                &scale,
                15.0,
                Weight::REGULAR,
                0.0,
            ),
        ];
        self.clamp_scroll();
        self.scroll = self.scroll_target;
    }

    /// Replaces the rows (same order as before, or the selection is the caller's to fix).
    pub fn set_rows(&mut self, fonts: &Fonts, views: &[RowView<'_>]) {
        let s = self.scale;
        let old = std::mem::take(&mut self.rows);
        self.rows = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let initial: String = v
                    .name
                    .chars()
                    .find(|c| !c.is_whitespace())
                    .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect());
                let (hover, selected) = old
                    .get(i)
                    .map_or_else(Default::default, |r| (r.hover, r.selected));
                Row {
                    name: Text::new(fonts, v.name, &s, 17.0, Weight::SEMIBOLD, 0.0),
                    initial: Text::new(fonts, &initial, &s, 26.0, Weight::EXTRABOLD, 0.0),
                    motd: v
                        .motd
                        .lines()
                        .iter()
                        .take(2)
                        .map(|l| runs(fonts, &s, l, 14.5, MOTD_DEFAULT))
                        .collect(),
                    status: runs(fonts, &s, v.status, 14.0, FAINT),
                    latency: match v.ping {
                        Ping::Latency(ms) => Some(Text::new(
                            fonts,
                            &format!("{ms} ms"),
                            &s,
                            12.5,
                            Weight::MEDIUM,
                            0.0,
                        )),
                        _ => None,
                    },
                    ping: v.ping,
                    icon: v.icon,
                    hover,
                    selected,
                }
            })
            .collect();
        if self.selected.is_some_and(|i| i >= self.rows.len()) {
            self.selected = None;
        }
        self.clamp_scroll();
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects a row (or none) and scrolls it into view.
    pub fn select(&mut self, i: Option<usize>) {
        self.selected = i.filter(|&i| i < self.rows.len());
        if let Some(i) = self.selected {
            let top = i as f32 * (ROW_H + ROW_GAP);
            if top < self.scroll_target {
                self.scroll_target = top;
            } else if top + ROW_H > self.scroll_target + self.list.h {
                self.scroll_target = top + ROW_H - self.list.h;
            }
            self.clamp_scroll();
        }
    }

    /// Moves the selection by `delta` rows (from none: the first or last row).
    pub fn move_selection(&mut self, delta: isize) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None if delta > 0 => 0,
            None => n - 1,
            Some(i) => (i as isize + delta).clamp(0, n as isize - 1) as usize,
        };
        self.select(Some(next));
    }

    fn content_height(&self) -> f32 {
        let n = self.rows.len() as f32;
        (n * (ROW_H + ROW_GAP) - ROW_GAP).max(0.0)
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height() - self.list.h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll_target = self.scroll_target.clamp(0.0, self.max_scroll());
    }

    /// The wheel turned `notches` (positive scrolls up).
    pub fn wheel(&mut self, notches: f32) {
        self.scroll_target -= notches * WHEEL_STEP;
        self.clamp_scroll();
    }

    fn row_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.list.x,
            y: self.list.y + i as f32 * (ROW_H + ROW_GAP) - self.scroll,
            w: self.list.w,
            h: ROW_H,
        }
    }

    /// The row at `(x, y)` (UI units), if any.
    pub fn hit_row(&self, x: f32, y: f32) -> Option<usize> {
        if !self.list.contains(x, y) {
            return None;
        }
        (0..self.rows.len()).find(|&i| self.row_rect(i).contains(x, y))
    }

    /// The button at `(x, y)` (UI units), if any.
    pub fn hit_button(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.rect.contains(x, y))
    }

    /// The pointer moved to `(x, y)` (UI units), or left (`None`).
    pub fn pointer(&mut self, at: Option<(f32, f32)>) {
        self.hovered = at.and_then(|(x, y)| self.hit_row(x, y));
        self.states
            .hover(at.and_then(|(x, y)| self.hit_button(x, y)));
    }

    /// The left button went down at `(x, y)` at `time` seconds: selects the row under
    /// it, and returns true for the second click of a double click on the same row.
    pub fn click_row(&mut self, x: f32, y: f32, time: f64) -> bool {
        let Some(i) = self.hit_row(x, y) else {
            return false;
        };
        self.select(Some(i));
        let double = self
            .last_click
            .is_some_and(|(j, t)| j == i && time - t < DOUBLE_CLICK);
        self.last_click = if double { None } else { Some((i, time)) };
        double
    }

    /// Advances scrolling and the rows' motion by `dt` seconds; enables the buttons that
    /// need a selection.
    pub fn update(&mut self, dt: f32) {
        let k = 1.0 - (-dt.max(0.0) / SCROLL_TAU).exp();
        self.scroll += (self.scroll_target - self.scroll) * k;
        if (self.scroll - self.scroll_target).abs() < 0.05 {
            self.scroll = self.scroll_target;
        }
        for (i, r) in self.rows.iter_mut().enumerate() {
            let lit = f32::from(u8::from(self.hovered == Some(i)));
            r.hover.step(lit, dt, 0.06, 0.15);
            let sel = f32::from(u8::from(self.selected == Some(i)));
            r.selected.step(sel, dt, 0.05, 0.12);
        }
        let has = self.selected.is_some();
        for b in [JOIN, EDIT, DELETE] {
            self.states.set_enabled(b, has);
        }
        self.states.update(dt);
    }

    /// Adds the screen's shapes and text; `time` (seconds) animates pinging rows.
    pub fn draw(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        time: f64,
    ) {
        let s = &self.scale;
        if let Some(atlas) = atlas {
            let title = Rect {
                x: 0.0,
                y: 36.0,
                w: s.width,
                h: 40.0,
            };
            self.title.centred(text, atlas, s, title, WHITE_TEXT);
            if self.rows.is_empty() {
                let mid = self.list.y + 0.4 * self.list.h;
                let r0 = Rect {
                    x: self.list.x,
                    y: mid - 30.0,
                    w: self.list.w,
                    h: 30.0,
                };
                let r1 = Rect {
                    x: self.list.x,
                    y: mid + 4.0,
                    w: self.list.w,
                    h: 24.0,
                };
                self.empty[0].centred(text, atlas, s, r0, SOFT);
                self.empty[1].centred(text, atlas, s, r1, FAINT);
            }
        }
        // Rows, clipped to the list (with room for the selection ring), fading at an edge
        // only while there is more to scroll to beyond it.
        let area = self.list.outset(8.0);
        let fade_top = self.scroll.min(FADE) * s.px;
        let fade_bottom = (self.max_scroll() - self.scroll).clamp(0.0, FADE) * s.px;
        let clip = Clip {
            rect: px_rect(s, area),
            fade_top,
            fade_bottom,
            opacity: 1.0,
        };
        shapes.set_clip(clip);
        text.set_clip(clip);
        for (i, row) in self.rows.iter().enumerate() {
            let r = self.row_rect(i);
            if r.y + r.h < area.y || r.y > area.y + area.h {
                continue;
            }
            self.draw_row(shapes, text, atlas, row, r, i, time, clip);
        }
        shapes.set_clip(Clip::NONE);
        text.set_clip(Clip::NONE);
        for (i, (b, label)) in self.buttons.iter().zip(&self.labels).enumerate() {
            draw_button(
                shapes,
                text,
                atlas,
                s,
                b.rect,
                label,
                self.states.look(i),
                self.states.enabled(i),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        row: &Row,
        r: Rect,
        index: usize,
        time: f64,
        clip: Clip,
    ) {
        let s = &self.scale;
        let tint = mix(
            mix(ROW_TINT, ROW_HOVER, row.hover.value),
            ROW_SELECTED,
            row.selected.value,
        );
        shapes.glass_rect(
            s,
            r,
            ROW_RADIUS,
            tint,
            0.55 + 0.3 * row.hover.value.max(row.selected.value),
        );
        draw_focus_ring(shapes, s, r, ROW_RADIUS, row.selected.value);
        // Icon, or a tile with the name's first letter.
        let icon = Rect {
            x: r.x + PAD,
            y: r.y + PAD,
            w: ICON,
            h: ICON,
        };
        match row.icon {
            Some(t) => shapes.image_rounded(s, icon, ICON_RADIUS, t),
            None => shapes.rounded(s, icon, ICON_RADIUS, [0.75, 0.8, 1.0, 0.10]),
        }
        // Signal bars, top right.
        let bars_right = r.x + r.w - PAD - 6.0;
        let bars_left = bars_right - (5.0 * 4.0 + 4.0 * 2.5);
        let base = r.y + 31.0;
        let lit = match row.ping {
            Ping::Latency(ms) => match ms {
                0..150 => 5,
                150..300 => 4,
                300..600 => 3,
                600..1000 => 2,
                _ => 1,
            },
            _ => 0,
        };
        for b in 0..5 {
            let h = 5.0 + 2.5 * b as f32;
            let bar = Rect {
                x: bars_left + b as f32 * 6.5,
                y: base - h,
                w: 4.0,
                h,
            };
            let color = match row.ping {
                Ping::Latency(_) if b < lit => match lit {
                    4 | 5 => BAR_GOOD,
                    3 => BAR_FAIR,
                    _ => BAR_POOR,
                },
                Ping::Pinging => {
                    // A wave running across the bars, offset per row as vanilla does.
                    let phase = time as f32 * 5.0 - b as f32 * 0.9 + index as f32 * 1.7;
                    let wave = 0.5 + 0.5 * phase.sin();
                    [1.0, 1.0, 1.0, 0.12 + 0.5 * wave * wave]
                }
                Ping::Failed | Ping::Incompatible => BAR_FAILED,
                Ping::Latency(_) => BAR_OFF,
            };
            shapes.rounded(s, bar, 1.5, color);
        }
        let Some(atlas) = atlas else { return };
        if row.icon.is_none() {
            row.initial
                .centred(text, atlas, s, icon, [0.9, 0.92, 1.0, 0.55]);
        }
        let x = r.x + TEXT_X;
        // The name and message stop short of the right-hand column.
        let column = |right: f32| Clip {
            rect: [clip.rect[0], clip.rect[1], right * s.px, clip.rect[3]],
            ..clip
        };
        let mut name_right = bars_left - 8.0;
        if let Some(l) = &row.latency {
            let lx = bars_left - 8.0 - l.width(s);
            l.draw_at(text, atlas, s, lx, base, FAINT);
            name_right = lx - 10.0;
        }
        text.set_clip(column(name_right));
        row.name.draw_at(text, atlas, s, x, base, WHITE_TEXT);
        let status_w = runs_width(&row.status, s);
        let status_x = r.x + r.w - PAD - 6.0 - status_w;
        let motd_right = (r.x + r.w - RIGHT).min(status_x - 12.0).max(x + 40.0);
        text.set_clip(column(motd_right));
        shapes.set_clip(column(motd_right));
        for (k, line) in row.motd.iter().enumerate() {
            draw_runs(
                shapes,
                text,
                atlas,
                s,
                line,
                x,
                r.y + 54.0 + 19.0 * k as f32,
            );
        }
        text.set_clip(clip);
        shapes.set_clip(clip);
        draw_runs(shapes, text, atlas, s, &row.status, status_x, r.y + 54.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modern::WEIGHTS;

    fn rows(n: usize) -> (Vec<StyledText>, StyledText) {
        let motd: Vec<StyledText> = (0..n)
            .map(|i| StyledText::literal(&format!("§aServer {i}\n§7second line"), Style::default()))
            .collect();
        (motd, StyledText::literal("3 / 100", Style::default()))
    }

    #[test]
    fn rows_scroll_and_select() {
        let fonts = Fonts::new(&WEIGHTS);
        let s = UiScale::new(1600, 900);
        let mut screen = ServerListScreen::new(s, &fonts);
        let (motd, status) = rows(20);
        let views: Vec<RowView> = motd
            .iter()
            .map(|m| RowView {
                name: "A server",
                motd: m,
                status: &status,
                ping: Ping::Latency(40),
                icon: None,
            })
            .collect();
        screen.set_rows(&fonts, &views);
        assert!(screen.max_scroll() > 0.0);
        // Selecting the last row scrolls it into view.
        screen.select(Some(19));
        screen.update(1.0);
        let last = screen.row_rect(19);
        assert!(last.y + last.h <= screen.list.y + screen.list.h + 0.01);
        assert!(screen.hit_row(last.x + 10.0, last.y + 10.0) == Some(19));
        // Keyboard moves clamp; the wheel scrolls back up.
        screen.move_selection(5);
        assert_eq!(screen.selected(), Some(19));
        screen.wheel(100.0);
        screen.update(1.0);
        assert_eq!(screen.row_rect(0).y, screen.list.y);
        // Join, Edit and Delete need a selection.
        screen.select(None);
        screen.update(0.0);
        assert!(!screen.states.enabled(JOIN) && screen.states.enabled(ADD));
    }

    #[test]
    fn double_click_joins() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = ServerListScreen::new(UiScale::new(1600, 900), &fonts);
        let (motd, status) = rows(2);
        let views: Vec<RowView> = motd
            .iter()
            .map(|m| RowView {
                name: "x",
                motd: m,
                status: &status,
                ping: Ping::Pinging,
                icon: None,
            })
            .collect();
        screen.set_rows(&fonts, &views);
        let r = screen.row_rect(1);
        let (x, y) = (r.x + 5.0, r.y + 5.0);
        assert!(!screen.click_row(x, y, 10.0));
        assert!(screen.click_row(x, y, 10.2));
        assert!(!screen.click_row(x, y, 10.3), "a third click starts over");
        assert!(!screen.click_row(x, y, 11.0));
        assert_eq!(screen.selected(), Some(1));
    }

    #[test]
    fn draws_rows_inside_the_list() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = ServerListScreen::new(UiScale::new(1600, 900), &fonts);
        let (motd, status) = rows(3);
        let views: Vec<RowView> = motd
            .iter()
            .map(|m| RowView {
                name: "x",
                motd: m,
                status: &status,
                ping: Ping::Latency(500),
                icon: Some(7),
            })
            .collect();
        screen.set_rows(&fonts, &views);
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        screen.draw(&mut shapes, &mut text, None, 0.0);
        let images: Vec<_> = shapes
            .shapes()
            .iter()
            .filter(|s| s.image.is_some())
            .collect();
        assert_eq!(images.len(), 3);
        assert!(images.iter().all(|s| s.clip.rect[1] < s.rect[1]));
        // Buttons come last and are not clipped.
        assert_eq!(shapes.shapes().last().unwrap().clip, Clip::NONE);
    }
}
