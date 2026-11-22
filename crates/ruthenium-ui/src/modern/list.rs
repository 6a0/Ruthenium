//! A vertical list of equal rows inside an area: smooth wheel scrolling, selection kept
//! in view, hover, double clicks, and the clip that fades an edge while there is more
//! beyond it. Screens keep their rows' content and draw them at [`ScrollList::row_rect`].

use super::{Rect, UiScale, px_rect};
use crate::Clip;

/// Units per wheel notch and the time constant of the glide.
const WHEEL_STEP: f32 = 64.0;
const SCROLL_TAU: f32 = 0.07;
/// Two clicks on one row this close together (seconds) make a double click (vanilla's
/// 250 ms).
const DOUBLE_CLICK: f64 = 0.25;
/// How far the edges fade (UI units).
const FADE: f32 = 28.0;

/// See the module docs.
#[derive(Debug, Clone)]
pub struct ScrollList {
    /// Where the rows show (UI units).
    pub area: Rect,
    row_h: f32,
    gap: f32,
    len: usize,
    selected: Option<usize>,
    hovered: Option<usize>,
    scroll: f32,
    scroll_target: f32,
    /// The last click on a row: which and when (seconds).
    last_click: Option<(usize, f64)>,
}

impl ScrollList {
    pub fn new(row_h: f32, gap: f32) -> ScrollList {
        ScrollList {
            area: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            row_h,
            gap,
            len: 0,
            selected: None,
            hovered: None,
            scroll: 0.0,
            scroll_target: 0.0,
            last_click: None,
        }
    }

    /// Moves the list to `area` (a new layout), keeping selection and scrolling.
    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
        self.clamp_scroll();
        self.scroll = self.scroll_target;
    }

    /// The number of rows changed; a selection past the end is dropped.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        if self.selected.is_some_and(|i| i >= len) {
            self.selected = None;
        }
        if self.hovered.is_some_and(|i| i >= len) {
            self.hovered = None;
        }
        self.clamp_scroll();
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Selects a row (or none) and scrolls it into view.
    pub fn select(&mut self, i: Option<usize>) {
        self.selected = i.filter(|&i| i < self.len);
        if let Some(i) = self.selected {
            let top = i as f32 * (self.row_h + self.gap);
            if top < self.scroll_target {
                self.scroll_target = top;
            } else if top + self.row_h > self.scroll_target + self.area.h {
                self.scroll_target = top + self.row_h - self.area.h;
            }
            self.clamp_scroll();
        }
    }

    /// Moves the selection by `delta` rows (from none: the first or last row).
    pub fn move_selection(&mut self, delta: isize) {
        let n = self.len;
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
        (self.len as f32 * (self.row_h + self.gap) - self.gap).max(0.0)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - self.area.h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll_target = self.scroll_target.clamp(0.0, self.max_scroll());
    }

    /// The wheel turned `notches` (positive scrolls up).
    pub fn wheel(&mut self, notches: f32) {
        self.scroll_target -= notches * WHEEL_STEP;
        self.clamp_scroll();
    }

    /// Row `i` where it shows now (UI units), scrolled.
    pub fn row_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.area.x,
            y: self.area.y + i as f32 * (self.row_h + self.gap) - self.scroll,
            w: self.area.w,
            h: self.row_h,
        }
    }

    /// The row at `(x, y)` (UI units), if any.
    pub fn hit_row(&self, x: f32, y: f32) -> Option<usize> {
        if !self.area.contains(x, y) {
            return None;
        }
        (0..self.len).find(|&i| self.row_rect(i).contains(x, y))
    }

    /// The pointer moved to `(x, y)` (UI units), or left (`None`).
    pub fn pointer(&mut self, at: Option<(f32, f32)>) {
        self.hovered = at.and_then(|(x, y)| self.hit_row(x, y));
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

    /// Advances scrolling by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let k = 1.0 - (-dt.max(0.0) / SCROLL_TAU).exp();
        self.scroll += (self.scroll_target - self.scroll) * k;
        if (self.scroll - self.scroll_target).abs() < 0.05 {
            self.scroll = self.scroll_target;
        }
    }

    /// The clip for the rows: the area grown by `margin` (room for a selection ring),
    /// fading at an edge only while there is more to scroll to beyond it.
    pub fn clip(&self, s: &UiScale, margin: f32) -> Clip {
        Clip {
            rect: px_rect(s, self.area.outset(margin)),
            fade_top: self.scroll.min(FADE) * s.px,
            fade_bottom: (self.max_scroll() - self.scroll).clamp(0.0, FADE) * s.px,
            opacity: 1.0,
        }
    }

    /// Whether row `r` (from [`Self::row_rect`]) shows within `margin` of the area.
    pub fn shows(&self, r: Rect, margin: f32) -> bool {
        let a = self.area.outset(margin);
        r.y + r.h >= a.y && r.y <= a.y + a.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_scrolls_into_view_and_clamps() {
        let mut l = ScrollList::new(50.0, 10.0);
        l.set_area(Rect {
            x: 0.0,
            y: 100.0,
            w: 300.0,
            h: 200.0,
        });
        l.set_len(10);
        assert_eq!(l.max_scroll(), 10.0 * 60.0 - 10.0 - 200.0);
        l.select(Some(9));
        l.update(1.0);
        let last = l.row_rect(9);
        assert!(last.y + last.h <= l.area.y + l.area.h + 0.01);
        assert_eq!(l.hit_row(10.0, last.y + 5.0), Some(9));
        l.move_selection(3);
        assert_eq!(l.selected(), Some(9));
        l.wheel(100.0);
        l.update(1.0);
        assert_eq!(l.row_rect(0).y, l.area.y);
        // Fewer rows drop a selection past the end.
        l.set_len(3);
        assert_eq!(l.selected(), None);
        assert_eq!(l.max_scroll(), 0.0);
        let mut empty = ScrollList::new(50.0, 10.0);
        empty.move_selection(1);
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn double_clicks_need_the_same_row_quickly() {
        let mut l = ScrollList::new(50.0, 10.0);
        l.set_area(Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 400.0,
        });
        l.set_len(2);
        let (x, y) = (5.0, 65.0);
        assert!(!l.click_row(x, y, 10.0));
        assert!(l.click_row(x, y, 10.2));
        assert!(!l.click_row(x, y, 10.3), "a third click starts over");
        assert!(!l.click_row(x, y, 11.0));
        assert_eq!(l.selected(), Some(1));
        assert!(!l.click_row(x, 500.0, 11.1), "outside the area");
    }
}
