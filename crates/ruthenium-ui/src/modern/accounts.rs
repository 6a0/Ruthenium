//! The Accounts screen: the Microsoft accounts signed in, as glass rows with the
//! player's face, name and (for the account in use) how its sign-in went; buttons to
//! add an account, switch to the selected one, sign it out, and go back.
//!
//! The screen keeps layout, shaped text, scrolling, selection and hover; the client keeps
//! the accounts and hands them over as [`AccountView`]s whenever they change.

use super::list::ScrollList;
use super::{
    Action, Button, ButtonStates, Ease, FAINT, Presence, Rect, SOFT, ShapeList, Text, UiScale,
    WHITE_TEXT, draw_button, draw_focus_ring, mix,
};
use crate::Clip;
use crate::text::{Fonts, GlyphAtlas, TextList, Weight};

/// One account as the list shows it.
#[derive(Debug, Clone, Copy)]
pub struct AccountView<'a> {
    pub name: &'a str,
    /// The account in use (only one).
    pub current: bool,
    /// How its sign-in went, for the account in use.
    pub detail: Option<(&'a str, Presence)>,
}

/// Button indices, in tab order.
pub const ADD: usize = 0;
pub const USE: usize = 1;
pub const SIGN_OUT: usize = 2;
pub const BACK: usize = 3;

const LIST_WIDTH: f32 = 560.0;
const ROW_H: f32 = 72.0;
const ROW_GAP: f32 = 10.0;
const ROW_RADIUS: f32 = 18.0;
const PAD: f32 = 14.0;
const AVATAR: f32 = 44.0;
const AVATAR_RADIUS: f32 = 10.0;
const TEXT_X: f32 = PAD + AVATAR + 16.0;
const DOT: f32 = 7.0;
const BUTTON_H: f32 = 44.0;
const BUTTON_GAP: f32 = 12.0;
const FOOTER_MARGIN: f32 = 30.0;
/// Space between the list and the buttons, where the note goes.
const NOTE_H: f32 = 40.0;

const ROW_TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.03];
const ROW_HOVER: [f32; 4] = [0.90, 0.93, 1.0, 0.08];
const ROW_SELECTED: [f32; 4] = [0.80, 0.86, 1.0, 0.14];
const AVATAR_TINT: [f32; 4] = [0.75, 0.8, 1.0, 0.10];
const BADGE: [f32; 4] = [0.62, 0.74, 1.0, 0.16];

#[derive(Debug, Clone)]
struct Row {
    name: Text,
    initial: Text,
    /// The player's face (a texture index), once on the GPU; the initial until then.
    face: Option<u32>,
    detail: Option<(Text, Presence)>,
    current: bool,
    hover: Ease,
    selected: Ease,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct AccountsScreen {
    pub scale: UiScale,
    pub buttons: Vec<Button>,
    pub states: ButtonStates,
    /// The rows' area, scrolling and selection.
    pub list: ScrollList,
    labels: Vec<Text>,
    title: Text,
    empty: [Text; 2],
    note: Text,
    badge: Text,
    rows: Vec<Row>,
}

impl AccountsScreen {
    pub fn new(scale: UiScale, fonts: &Fonts) -> AccountsScreen {
        let entries = [
            ("Add Account", Action::AddAccount),
            ("Use", Action::UseAccount),
            ("Sign Out", Action::SignOut),
            ("Back", Action::Back),
        ];
        let zero = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        let blank = || Text::new(fonts, "", &scale, 12.0, Weight::REGULAR, 0.0);
        let mut screen = AccountsScreen {
            scale,
            buttons: entries
                .iter()
                .map(|&(label, action)| Button {
                    rect: zero,
                    label,
                    action,
                })
                .collect(),
            states: ButtonStates::new(entries.len()),
            list: ScrollList::new(ROW_H, ROW_GAP),
            labels: Vec::new(),
            title: blank(),
            empty: [blank(), blank()],
            note: blank(),
            badge: blank(),
            rows: Vec::new(),
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
        let y0 = scale.height - FOOTER_MARGIN - BUTTON_H;
        let top = 104.0;
        self.list.set_area(Rect {
            x: cx - 0.5 * w,
            y: top,
            w,
            h: (y0 - NOTE_H - top).max(ROW_H),
        });
        let n = self.buttons.len() as f32;
        let bw = (w - (n - 1.0) * BUTTON_GAP) / n;
        for (i, b) in self.buttons.iter_mut().enumerate() {
            b.rect = Rect {
                x: cx - 0.5 * w + i as f32 * (bw + BUTTON_GAP),
                y: y0,
                w: bw,
                h: BUTTON_H,
            };
        }
        self.labels = self
            .buttons
            .iter()
            .map(|b| Text::new(fonts, b.label, &scale, 16.0, Weight::SEMIBOLD, 0.01))
            .collect();
        self.title = Text::new(fonts, "Accounts", &scale, 28.0, Weight::SEMIBOLD, 0.01);
        self.empty = [
            Text::new(
                fonts,
                "No accounts yet",
                &scale,
                20.0,
                Weight::SEMIBOLD,
                0.0,
            ),
            Text::new(
                fonts,
                "Sign in with a Microsoft account that owns Minecraft: Java Edition.",
                &scale,
                15.0,
                Weight::REGULAR,
                0.0,
            ),
        ];
        self.note = Text::new(
            fonts,
            "Sign-ins are kept in your system's credential store, never in Ruthenium's files.",
            &scale,
            12.5,
            Weight::REGULAR,
            0.0,
        );
        self.badge = Text::new(fonts, "IN USE", &scale, 11.0, Weight::EXTRABOLD, 0.08);
    }

    /// Replaces the rows; the selection stays on the same index.
    pub fn set_rows(&mut self, fonts: &Fonts, views: &[AccountView<'_>]) {
        let s = self.scale;
        let old = std::mem::take(&mut self.rows);
        self.rows = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let initial: String = v
                    .name
                    .chars()
                    .next()
                    .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect());
                let (hover, selected, face) = old
                    .get(i)
                    .map_or((Ease::default(), Ease::default(), None), |r| {
                        (r.hover, r.selected, r.face)
                    });
                Row {
                    name: Text::new(fonts, v.name, &s, 17.0, Weight::SEMIBOLD, 0.0),
                    initial: Text::new(fonts, &initial, &s, 20.0, Weight::EXTRABOLD, 0.0),
                    detail: v
                        .detail
                        .map(|(d, p)| (Text::new(fonts, d, &s, 14.0, Weight::REGULAR, 0.0), p)),
                    current: v.current,
                    face,
                    hover,
                    selected,
                }
            })
            .collect();
        self.list.set_len(self.rows.len());
    }

    /// Each row's face texture (in row order; `None`: not on the GPU, the initial shows).
    pub fn set_faces(&mut self, faces: impl IntoIterator<Item = Option<u32>>) {
        for (row, face) in self.rows.iter_mut().zip(faces) {
            row.face = face;
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected()
    }

    /// Selects a row (or none) and scrolls it into view.
    pub fn select(&mut self, i: Option<usize>) {
        self.list.select(i);
    }

    /// The button at `(x, y)` (UI units), if any.
    pub fn hit_button(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.rect.contains(x, y))
    }

    /// The pointer moved to `(x, y)` (UI units), or left (`None`).
    pub fn pointer(&mut self, at: Option<(f32, f32)>) {
        self.list.pointer(at);
        self.states
            .hover(at.and_then(|(x, y)| self.hit_button(x, y)));
    }

    /// Advances scrolling and the rows' motion by `dt` seconds; enables the buttons that
    /// need a selection (Use only for an account not in use).
    pub fn update(&mut self, dt: f32) {
        self.list.update(dt);
        for (i, r) in self.rows.iter_mut().enumerate() {
            let lit = f32::from(u8::from(self.list.hovered() == Some(i)));
            r.hover.step(lit, dt, 0.06, 0.15);
            let sel = f32::from(u8::from(self.list.selected() == Some(i)));
            r.selected.step(sel, dt, 0.05, 0.12);
        }
        let selected = self.list.selected().and_then(|i| self.rows.get(i));
        self.states
            .set_enabled(USE, selected.is_some_and(|r| !r.current));
        self.states.set_enabled(SIGN_OUT, selected.is_some());
        self.states.update(dt);
    }

    /// Adds the screen's shapes and text.
    pub fn draw(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: Option<&GlyphAtlas>) {
        let s = &self.scale;
        let area = self.list.area;
        if let Some(atlas) = atlas {
            let title = Rect {
                x: 0.0,
                y: 36.0,
                w: s.width,
                h: 40.0,
            };
            self.title.centred(text, atlas, s, title, WHITE_TEXT);
            if self.rows.is_empty() {
                let mid = area.y + 0.35 * area.h;
                let line = |y: f32, h: f32| Rect {
                    x: 0.0,
                    y,
                    w: s.width,
                    h,
                };
                self.empty[0].centred(text, atlas, s, line(mid - 30.0, 30.0), SOFT);
                self.empty[1].centred(text, atlas, s, line(mid + 4.0, 24.0), FAINT);
            }
            let note = Rect {
                x: 0.0,
                y: area.y + area.h,
                w: s.width,
                h: NOTE_H,
            };
            self.note.centred(text, atlas, s, note, FAINT);
        }
        let clip = self.list.clip(s, 8.0);
        shapes.set_clip(clip);
        text.set_clip(clip);
        for (i, row) in self.rows.iter().enumerate() {
            let r = self.list.row_rect(i);
            if self.list.shows(r, 8.0) {
                self.draw_row(shapes, text, atlas, row, r);
            }
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

    fn draw_row(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        row: &Row,
        r: Rect,
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
        let avatar = Rect {
            x: r.x + PAD,
            y: r.y + 0.5 * (r.h - AVATAR),
            w: AVATAR,
            h: AVATAR,
        };
        match row.face {
            Some(t) => shapes.image_rounded(s, avatar, AVATAR_RADIUS, t),
            None => shapes.rounded(s, avatar, AVATAR_RADIUS, AVATAR_TINT),
        }
        let badge = row.current.then(|| {
            let w = self.badge.width(s) + 20.0;
            Rect {
                x: r.x + r.w - PAD - 4.0 - w,
                y: r.y + 0.5 * (r.h - 24.0),
                w,
                h: 24.0,
            }
        });
        if let Some(b) = badge {
            shapes.rounded(s, b, 12.0, BADGE);
        }
        let x = r.x + TEXT_X;
        let detail_y = r.y + 50.0;
        if let Some((_, p)) = &row.detail {
            let dot = Rect {
                x,
                y: detail_y - 0.5 * DOT - 4.5,
                w: DOT,
                h: DOT,
            };
            shapes.rounded(s, dot, 0.5 * DOT, p.color());
        }
        let Some(atlas) = atlas else { return };
        if row.face.is_none() {
            row.initial
                .centred(text, atlas, s, avatar, [0.9, 0.92, 1.0, 0.6]);
        }
        if let Some(b) = badge {
            self.badge.centred(text, atlas, s, b, [0.86, 0.9, 1.0, 0.9]);
        }
        // Text stops short of the badge.
        let right = badge.map_or(r.x + r.w - PAD, |b| b.x - 10.0);
        let clip = self.list.clip(s, 8.0);
        text.set_clip(Clip {
            rect: [clip.rect[0], clip.rect[1], right * s.px, clip.rect[3]],
            ..clip
        });
        match &row.detail {
            Some((d, _)) => {
                row.name.draw_at(text, atlas, s, x, r.y + 30.0, WHITE_TEXT);
                d.draw_at(text, atlas, s, x + DOT + 8.0, detail_y, SOFT);
            }
            None => row
                .name
                .left_in(text, atlas, s, Rect { x, ..r }, WHITE_TEXT),
        }
        text.set_clip(clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modern::WEIGHTS;

    fn views<'a>(names: &'a [&'a str]) -> Vec<AccountView<'a>> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| AccountView {
                name: n,
                current: i == 0,
                detail: (i == 0).then_some(("Signed in", Presence::SignedIn)),
            })
            .collect()
    }

    #[test]
    fn use_needs_another_account_and_sign_out_a_selection() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = AccountsScreen::new(UiScale::new(1600, 900), &fonts);
        screen.set_rows(&fonts, &views(&["Alex", "Steve"]));
        screen.update(0.0);
        assert!(screen.states.enabled(ADD) && screen.states.enabled(BACK));
        assert!(!screen.states.enabled(USE) && !screen.states.enabled(SIGN_OUT));
        screen.select(Some(0));
        screen.update(0.0);
        assert!(!screen.states.enabled(USE), "already in use");
        assert!(screen.states.enabled(SIGN_OUT));
        screen.select(Some(1));
        screen.update(0.0);
        assert!(screen.states.enabled(USE));
        // Fewer rows: the selection past the end goes.
        screen.set_rows(&fonts, &views(&["Alex"]));
        assert_eq!(screen.selected(), None);
    }

    #[test]
    fn rows_sit_in_the_list_above_the_buttons() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut screen = AccountsScreen::new(UiScale::new(1280, 720), &fonts);
        screen.set_rows(&fonts, &views(&["Alex", "Steve", "Notch"]));
        let area = screen.list.area;
        assert!(area.y + area.h < screen.buttons[ADD].rect.y);
        let r = screen.list.row_rect(1);
        assert_eq!(screen.list.hit_row(r.x + 4.0, r.y + 4.0), Some(1));
        assert_eq!(
            screen.hit_button(
                screen.buttons[BACK].rect.x + 2.0,
                screen.buttons[BACK].rect.y + 2.0
            ),
            Some(BACK)
        );
        let mut shapes = ShapeList::new();
        let mut text = TextList::new();
        screen.draw(&mut shapes, &mut text, None);
        // Three rows (glass, avatar), one badge and dot for the account in use, four
        // buttons; the buttons are not clipped.
        let glass = shapes.shapes().iter().filter(|s| s.glass).count();
        assert_eq!(glass, 3 + 4);
        assert_eq!(shapes.shapes().last().unwrap().clip, Clip::NONE);
        // Faces replace the initials' tiles, and survive new rows.
        screen.set_faces([Some(5), None, Some(6)]);
        screen.set_rows(&fonts, &views(&["Alex", "Steve", "Notch"]));
        shapes.clear();
        screen.draw(&mut shapes, &mut text, None);
        let faces: Vec<u32> = shapes
            .shapes()
            .iter()
            .filter_map(|s| s.image.map(|i| i.texture))
            .collect();
        assert_eq!(faces, [5, 6]);
    }
}
