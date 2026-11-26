//! The server list (Multiplayer): saved servers as glass rows with their icon, name,
//! message of the day, player count and latency, in a scrolling list; under them a row
//! saying the local network is being scanned, and the games found there; the buttons
//! below. Hovering a player count shows who is online, as the server tells.
//!
//! The screen keeps layout, shaped text, scrolling, selection and hover; the client keeps
//! the servers and their ping results and hands them over as [`RowView`]s whenever they
//! change.

use ruthenium_protocol::text::{Style, StyledText};

use super::list::ScrollList;
use super::{
    Action, Button, ButtonStates, Ease, FAINT, Rect, SOFT, ShapeList, Text, UiScale, WHITE_TEXT,
    draw_button, draw_focus_ring, mix,
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
    /// Shown over the player count: the server's sample of who is online.
    pub players: &'a [StyledText],
    pub ping: Ping,
    /// The favicon's texture index, once uploaded.
    pub icon: Option<u32>,
}

/// A game found on the local network, as the list shows it.
#[derive(Debug, Clone, Copy)]
pub struct LanView<'a> {
    /// Its message (legacy `§` codes apply).
    pub motd: &'a StyledText,
    pub address: &'a str,
}

/// What a row of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A saved server, by its place in the list.
    Saved(usize),
    /// The line saying the local network is being scanned (`LANHeader`).
    Scanning,
    /// A game on the local network, by its place among them.
    Lan(usize),
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

/// Vanilla's `lanServer.scanning` and `lanServer.title`.
const SCANNING: &str = "Scanning for games on your local network";
const LAN_TITLE: &str = "LAN World";
/// The player count's tooltip: its padding, line height and gap from the list.
const TIP_PAD: f32 = 12.0;
const TIP_LINE: f32 = 19.0;
const TIP_GAP: f32 = 14.0;
const TIP_TINT: [f32; 4] = [0.80, 0.86, 1.0, 0.10];

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
    players: Vec<Vec<Run>>,
    latency: Option<Text>,
    ping: Ping,
    icon: Option<u32>,
    hover: Ease,
    selected: Ease,
}

/// A game on the local network's row.
#[derive(Debug, Clone)]
struct LanRow {
    motd: Vec<Run>,
    address: Text,
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
    /// The rows' area, scrolling and selection.
    pub list: ScrollList,
    labels: Vec<Text>,
    title: Text,
    empty: [Text; 2],
    rows: Vec<Row>,
    lan: Vec<LanRow>,
    /// The scanning line, and the LAN rows' title.
    scanning: Text,
    lan_title: Text,
    lan_tile: Text,
    /// Where the pointer is (UI units).
    pointer: Option<(f32, f32)>,
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
            list: ScrollList::new(ROW_H, ROW_GAP),
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
            lan: Vec::new(),
            scanning: Text::new(fonts, SCANNING, &scale, 14.0, Weight::MEDIUM, 0.0),
            lan_title: Text::new(fonts, LAN_TITLE, &scale, 17.0, Weight::SEMIBOLD, 0.0),
            lan_tile: Text::new(fonts, "LAN", &scale, 15.0, Weight::EXTRABOLD, 0.04),
            pointer: None,
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
        self.list.set_area(Rect {
            x: cx - 0.5 * w,
            y: 104.0,
            w,
            h: (bottom - 104.0).max(ROW_H),
        });
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
        self.scanning = Text::new(fonts, SCANNING, &scale, 14.0, Weight::MEDIUM, 0.0);
        self.lan_title = Text::new(fonts, LAN_TITLE, &scale, 17.0, Weight::SEMIBOLD, 0.0);
        self.lan_tile = Text::new(fonts, "LAN", &scale, 15.0, Weight::EXTRABOLD, 0.04);
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
                    players: v
                        .players
                        .iter()
                        .map(|p| runs(fonts, &s, p, 14.0, SOFT))
                        .collect(),
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
        self.list.set_len(self.len());
    }

    /// Replaces the games found on the local network (in the order they were found).
    pub fn set_lan(&mut self, fonts: &Fonts, views: &[LanView<'_>]) {
        let s = self.scale;
        let old = std::mem::take(&mut self.lan);
        self.lan = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let (hover, selected) = old
                    .get(i)
                    .map_or_else(Default::default, |r| (r.hover, r.selected));
                LanRow {
                    motd: v
                        .motd
                        .lines()
                        .first()
                        .map(|l| runs(fonts, &s, l, 14.5, MOTD_DEFAULT))
                        .unwrap_or_default(),
                    address: Text::new(fonts, v.address, &s, 13.5, Weight::REGULAR, 0.0),
                    hover,
                    selected,
                }
            })
            .collect();
        self.list.set_len(self.len());
    }

    /// Rows in the list: the saved servers, the scanning line, the LAN games.
    fn len(&self) -> usize {
        self.rows.len() + 1 + self.lan.len()
    }

    /// What row `i` is.
    pub fn entry(&self, i: usize) -> Option<Entry> {
        let saved = self.rows.len();
        match i {
            _ if i < saved => Some(Entry::Saved(i)),
            _ if i == saved => Some(Entry::Scanning),
            _ if i - saved - 1 < self.lan.len() => Some(Entry::Lan(i - saved - 1)),
            _ => None,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected()
    }

    /// What the selected row is.
    pub fn selected_entry(&self) -> Option<Entry> {
        self.entry(self.list.selected()?)
    }

    /// Selects a row (or none) and scrolls it into view.
    pub fn select(&mut self, i: Option<usize>) {
        self.list.select(i);
    }

    /// Moves the selection by `delta` rows (from none: the first or last row).
    pub fn move_selection(&mut self, delta: isize) {
        self.list.move_selection(delta);
    }

    /// The wheel turned `notches` (positive scrolls up).
    pub fn wheel(&mut self, notches: f32) {
        self.list.wheel(notches);
    }

    /// The button at `(x, y)` (UI units), if any.
    pub fn hit_button(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.rect.contains(x, y))
    }

    /// The pointer moved to `(x, y)` (UI units), or left (`None`).
    pub fn pointer(&mut self, at: Option<(f32, f32)>) {
        self.pointer = at;
        self.list.pointer(at);
        self.states
            .hover(at.and_then(|(x, y)| self.hit_button(x, y)));
    }

    /// The left button went down at `(x, y)` at `time` seconds: selects the row under
    /// it, and returns true for the second click of a double click on the same row.
    pub fn click_row(&mut self, x: f32, y: f32, time: f64) -> bool {
        self.list.click_row(x, y, time)
    }

    /// Advances scrolling and the rows' motion by `dt` seconds; enables the buttons that
    /// need a selection.
    pub fn update(&mut self, dt: f32) {
        self.list.update(dt);
        let lan_start = self.rows.len() + 1;
        let motion = self
            .rows
            .iter_mut()
            .map(|r| (&mut r.hover, &mut r.selected))
            .enumerate()
            .chain(
                self.lan
                    .iter_mut()
                    .map(|r| (&mut r.hover, &mut r.selected))
                    .enumerate()
                    .map(|(i, m)| (lan_start + i, m)),
            );
        for (i, (hover, selected)) in motion {
            let lit = f32::from(u8::from(self.list.hovered() == Some(i)));
            hover.step(lit, dt, 0.06, 0.15);
            let sel = f32::from(u8::from(self.list.selected() == Some(i)));
            selected.step(sel, dt, 0.05, 0.12);
        }
        // `JoinMultiplayerScreen.onSelectedChange`: any server joins; only saved ones edit.
        let entry = self.selected_entry();
        let saved = matches!(entry, Some(Entry::Saved(_)));
        self.states
            .set_enabled(JOIN, saved || matches!(entry, Some(Entry::Lan(_))));
        for b in [EDIT, DELETE] {
            self.states.set_enabled(b, saved);
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
            if self.rows.is_empty() && self.lan.is_empty() {
                let area = self.list.area;
                let mid = area.y + 0.4 * area.h;
                let r0 = Rect {
                    x: area.x,
                    y: mid - 30.0,
                    w: area.w,
                    h: 30.0,
                };
                let r1 = Rect {
                    x: area.x,
                    y: mid + 4.0,
                    w: area.w,
                    h: 24.0,
                };
                self.empty[0].centred(text, atlas, s, r0, SOFT);
                self.empty[1].centred(text, atlas, s, r1, FAINT);
            }
        }
        // Rows, clipped to the list (with room for the selection ring), fading at an edge
        // only while there is more to scroll to beyond it.
        let clip = self.list.clip(s, 8.0);
        shapes.set_clip(clip);
        text.set_clip(clip);
        for i in 0..self.len() {
            let r = self.list.row_rect(i);
            if !self.list.shows(r, 8.0) {
                continue;
            }
            match self.entry(i) {
                Some(Entry::Saved(k)) => {
                    self.draw_row(shapes, text, atlas, &self.rows[k], r, i, time, clip);
                }
                Some(Entry::Scanning) => self.draw_scanning(shapes, text, atlas, r, time),
                Some(Entry::Lan(k)) => self.draw_lan(shapes, text, atlas, &self.lan[k], r),
                None => {}
            }
        }
        shapes.set_clip(Clip::NONE);
        text.set_clip(Clip::NONE);
        if let Some(atlas) = atlas {
            self.draw_players(shapes, text, atlas);
        }
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

    /// The scanning line: faint, centred, with three dots that come and go
    /// (`LoadingDotsWidget`).
    fn draw_scanning(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        r: Rect,
        time: f64,
    ) {
        let s = &self.scale;
        let selected = self.list.selected() == self.list_index(Entry::Scanning);
        draw_focus_ring(shapes, s, r, ROW_RADIUS, f32::from(u8::from(selected)));
        let w = self.scanning.width(s);
        let dots = 3.0 * 9.0;
        let x = r.x + 0.5 * (r.w - w - dots);
        let mid = r.y + 0.5 * r.h;
        for k in 0..3 {
            let phase = time as f32 * 4.0 - k as f32 * 0.8;
            let a = 0.15 + 0.5 * (0.5 + 0.5 * phase.sin());
            let d = Rect {
                x: x + w + 8.0 + k as f32 * 9.0,
                y: mid - 2.5,
                w: 5.0,
                h: 5.0,
            };
            shapes.rounded(s, d, 2.5, [0.85, 0.87, 1.0, a]);
        }
        if let Some(atlas) = atlas {
            self.scanning
                .left_in(text, atlas, s, Rect { x, ..r }, FAINT);
        }
    }

    /// A LAN game's row: a tile, "LAN World", its message and its address.
    fn draw_lan(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        row: &LanRow,
        r: Rect,
    ) {
        let s = &self.scale;
        let tint = mix(
            mix(ROW_TINT, ROW_HOVER, row.hover.value),
            ROW_SELECTED,
            row.selected.value,
        );
        let lit = row.hover.value.max(row.selected.value);
        shapes.glass_rect(s, r, ROW_RADIUS, tint, 0.55 + 0.3 * lit);
        draw_focus_ring(shapes, s, r, ROW_RADIUS, row.selected.value);
        let icon = Rect {
            x: r.x + PAD,
            y: r.y + PAD,
            w: ICON,
            h: ICON,
        };
        shapes.rounded(s, icon, ICON_RADIUS, [0.45, 0.62, 1.0, 0.16]);
        let Some(atlas) = atlas else { return };
        self.lan_tile
            .centred(text, atlas, s, icon, [0.9, 0.92, 1.0, 0.7]);
        let x = r.x + TEXT_X;
        self.lan_title
            .draw_at(text, atlas, s, x, r.y + 31.0, WHITE_TEXT);
        let clip = text.clip();
        let right = (r.x + r.w - PAD) * s.px;
        text.set_clip(Clip {
            rect: [
                clip.rect[0],
                clip.rect[1],
                right.min(clip.rect[2]),
                clip.rect[3],
            ],
            ..clip
        });
        draw_runs(shapes, text, atlas, s, &row.motd, x, r.y + 54.0);
        row.address.draw_at(text, atlas, s, x, r.y + 73.0, FAINT);
        text.set_clip(clip);
    }

    /// The list index of `entry`.
    fn list_index(&self, entry: Entry) -> Option<usize> {
        match entry {
            Entry::Saved(i) => (i < self.rows.len()).then_some(i),
            Entry::Scanning => Some(self.rows.len()),
            Entry::Lan(i) => (i < self.lan.len()).then_some(self.rows.len() + 1 + i),
        }
    }

    /// The player count of saved server `k` (in row rect `r`): where it is drawn.
    fn status_rect(&self, row: &Row, r: Rect) -> Rect {
        let w = runs_width(&row.status, &self.scale);
        Rect {
            x: r.x + r.w - PAD - 6.0 - w,
            y: r.y + 40.0,
            w,
            h: 19.0,
        }
    }

    /// Who is online, beside the list, while the pointer is on a player count
    /// (`OnlineServerEntry`'s players tooltip). It stands outside the list so no other
    /// text is under it.
    fn draw_players(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: &GlyphAtlas) {
        let s = &self.scale;
        let Some((px, py)) = self.pointer else {
            return;
        };
        let area = self.list.area;
        if !area.contains(px, py) {
            return;
        }
        let hit = (0..self.rows.len()).find_map(|k| {
            let r = self.list.row_rect(k);
            let row = &self.rows[k];
            (!row.players.is_empty() && self.status_rect(row, r).contains(px, py))
                .then_some((row, r))
        });
        let Some((row, r)) = hit else {
            return;
        };
        let w = row
            .players
            .iter()
            .map(|l| runs_width(l, s))
            .fold(0.0, f32::max)
            + 2.0 * TIP_PAD;
        let h = row.players.len() as f32 * TIP_LINE + 2.0 * TIP_PAD - 4.0;
        let right = area.x + area.w + TIP_GAP;
        let x = if right + w <= s.width - 8.0 {
            right
        } else {
            (area.x - TIP_GAP - w).max(8.0)
        };
        let y = r.y.min(s.height - 8.0 - h).max(8.0);
        let tip = Rect { x, y, w, h };
        shapes.glass_rect(s, tip, 14.0, TIP_TINT, 0.6);
        for (i, line) in row.players.iter().enumerate() {
            let base = y + TIP_PAD + 10.0 + i as f32 * TIP_LINE;
            draw_runs(shapes, text, atlas, s, line, x + TIP_PAD, base);
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
                players: &[],
                ping: Ping::Latency(40),
                icon: None,
            })
            .collect();
        screen.set_rows(&fonts, &views);
        assert!(screen.list.max_scroll() > 0.0);
        // Selecting the last row scrolls it into view.
        screen.select(Some(19));
        screen.update(1.0);
        let last = screen.list.row_rect(19);
        let area = screen.list.area;
        assert!(last.y + last.h <= area.y + area.h + 0.01);
        assert!(screen.list.hit_row(last.x + 10.0, last.y + 10.0) == Some(19));
        // Keyboard moves clamp (at the scanning line, last); the wheel scrolls back up.
        screen.move_selection(5);
        assert_eq!(screen.selected(), Some(20));
        assert_eq!(screen.selected_entry(), Some(Entry::Scanning));
        screen.wheel(100.0);
        screen.update(1.0);
        assert_eq!(screen.list.row_rect(0).y, area.y);
        // Join, Edit and Delete need a selection.
        screen.select(None);
        screen.update(0.0);
        assert!(!screen.states.enabled(JOIN) && screen.states.enabled(ADD));
    }

    #[test]
    fn lan_games_follow_the_scanning_line() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = ServerListScreen::new(UiScale::new(1600, 900), &fonts);
        let (motd, status) = rows(2);
        let views: Vec<RowView> = motd
            .iter()
            .map(|m| RowView {
                name: "x",
                motd: m,
                status: &status,
                players: &[],
                ping: Ping::Pinging,
                icon: None,
            })
            .collect();
        screen.set_rows(&fonts, &views);
        let lan_motd = StyledText::literal("§aA ʟᴀɴ world", Style::default());
        screen.set_lan(
            &fonts,
            &[LanView {
                motd: &lan_motd,
                address: "192.168.1.5:25565",
            }],
        );
        assert_eq!(screen.list.len(), 4);
        assert_eq!(screen.entry(1), Some(Entry::Saved(1)));
        assert_eq!(screen.entry(2), Some(Entry::Scanning));
        assert_eq!(screen.entry(3), Some(Entry::Lan(0)));
        assert_eq!(screen.entry(4), None);
        // Saved servers join, edit and delete; LAN games only join; the line nothing.
        for (i, join, edit) in [(0, true, true), (2, false, false), (3, true, false)] {
            screen.select(Some(i));
            screen.update(0.0);
            assert_eq!(screen.states.enabled(JOIN), join, "{i}");
            assert_eq!(screen.states.enabled(EDIT), edit, "{i}");
            assert_eq!(screen.states.enabled(DELETE), edit, "{i}");
        }
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        let mut atlas = GlyphAtlas::new(1024);
        screen.draw(&mut shapes, &mut text, Some(&atlas), 0.0);
        // The atlas had none of the glyphs: every one asked for, the LAN row's too.
        let missing = text.take_missing();
        assert!(missing.len() > 20, "{}", missing.len());
        atlas.add_many(&fonts, &missing);
        text.clear();
        screen.draw(&mut shapes, &mut text, Some(&atlas), 0.0);
        assert!(text.take_missing().is_empty());
        // The address line is drawn in the LAN row.
        let lan = screen.list.row_rect(3);
        let s = screen.scale;
        assert!(text.quads().iter().any(|q| {
            let y = q.rect[3] / s.px;
            y > lan.y + 60.0 && y < lan.y + lan.h
        }));
    }

    #[test]
    fn the_player_count_shows_who_is_online() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = ServerListScreen::new(UiScale::new(1600, 900), &fonts);
        let motd = StyledText::literal("Hello", Style::default());
        let status = StyledText::literal("2 / 20", Style::default());
        let players = [
            StyledText::literal("§cᴀʟᴇx", Style::default()),
            StyledText::literal("Steve", Style::default()),
        ];
        screen.set_rows(
            &fonts,
            &[RowView {
                name: "x",
                motd: &motd,
                status: &status,
                players: &players,
                ping: Ping::Latency(20),
                icon: None,
            }],
        );
        let mut atlas = GlyphAtlas::new(2048);
        atlas.add_text(&fonts, Weight::REGULAR, "2 / 20 ᴀʟᴇx Steve Hello x");
        atlas.add_text(&fonts, Weight::SEMIBOLD, "x");
        let draw = |screen: &ServerListScreen| {
            let mut shapes = ShapeList::new();
            let mut text = TextList::new();
            screen.draw(&mut shapes, &mut text, Some(&atlas), 0.0);
            (shapes.shapes().len(), text.quads().len())
        };
        let r = screen.list.row_rect(0);
        let at = screen.status_rect(&screen.rows[0], r);
        screen.pointer(Some((at.x + 2.0, at.y + 0.5 * at.h)));
        let (shapes_on, glyphs_on) = draw(&screen);
        screen.pointer(Some((r.x + 5.0, r.y + 5.0)));
        let (shapes_off, glyphs_off) = draw(&screen);
        // A panel, and the two names' glyphs.
        assert_eq!(shapes_on, shapes_off + 1);
        assert_eq!(glyphs_on - glyphs_off, "ᴀʟᴇxSteve".chars().count());
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
                players: &[],
                ping: Ping::Pinging,
                icon: None,
            })
            .collect();
        screen.set_rows(&fonts, &views);
        let r = screen.list.row_rect(1);
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
                players: &[],
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
