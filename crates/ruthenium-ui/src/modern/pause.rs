//! The Game Menu over the world: the logo above vanilla's arrangement of buttons (a wide
//! one, a row of two, a wide one), as glass pills.

use super::{
    Action, Button, ButtonStates, LABEL_SIZE, Rect, ShapeList, Text, UiScale, draw_button,
};
use crate::text::{Fonts, GlyphAtlas, TextList, Weight};

const LOGO: f32 = 112.0;
/// From the logo to the buttons.
const SPACE: f32 = 40.0;
/// Above the logo, at least.
const MARGIN: f32 = 12.0;
const WIDTH: f32 = 460.0;
const HEIGHT: f32 = 52.0;
const GAP: f32 = 14.0;

/// See the module docs.
#[derive(Debug, Clone)]
pub struct PauseScreen {
    pub scale: UiScale,
    pub states: ButtonStates,
    pub buttons: Vec<Button>,
    labels: Vec<Text>,
    logo: Rect,
}

impl PauseScreen {
    pub fn new(scale: UiScale, fonts: &Fonts) -> PauseScreen {
        let mut screen = PauseScreen {
            scale,
            states: ButtonStates::new(4),
            buttons: Vec::new(),
            labels: Vec::new(),
            logo: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        };
        screen.layout(scale, fonts);
        screen
    }

    /// Lays the screen out for `scale`, keeping focus and motion.
    pub fn layout(&mut self, scale: UiScale, fonts: &Fonts) {
        self.scale = scale;
        let cx = 0.5 * scale.width;
        // The window's middle is the middle of the gap in the row of two.
        let top = 0.5 * scale.height - 1.5 * HEIGHT - GAP;
        // A window too low for all of it gets a smaller logo.
        let logo = LOGO.min(top - SPACE - MARGIN).max(0.0);
        self.logo = Rect {
            x: cx - 0.5 * logo,
            y: top - SPACE - logo,
            w: logo,
            h: logo,
        };
        let left = cx - 0.5 * WIDTH;
        let half = 0.5 * (WIDTH - GAP);
        let row = |i: f32| top + i * (HEIGHT + GAP);
        let button = |label, action, x, y, w| Button {
            rect: Rect { x, y, w, h: HEIGHT },
            label,
            action,
        };
        self.buttons = vec![
            button("Back to Game", Action::Back, left, row(0.0), WIDTH),
            button("Ruthenium Options", Action::Settings, left, row(1.0), half),
            button(
                "MC Options",
                Action::GameSettings,
                left + half + GAP,
                row(1.0),
                half,
            ),
            button("Disconnect", Action::Cancel, left, row(2.0), WIDTH),
        ];
        self.labels = self
            .buttons
            .iter()
            .map(|b| Text::new(fonts, b.label, &scale, LABEL_SIZE, Weight::SEMIBOLD, 0.01))
            .collect();
    }

    /// The index of the button at `(x, y)` (UI units).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.rect.contains(x, y))
    }

    /// Adds the screen's shapes and, once they are on the GPU, its text (`atlas`) and the
    /// logo (a texture index).
    pub fn draw(
        &self,
        shapes: &mut ShapeList,
        text: &mut TextList,
        atlas: Option<&GlyphAtlas>,
        logo: Option<u32>,
    ) {
        let s = &self.scale;
        if let Some(logo) = logo {
            shapes.image(s, self.logo, logo);
        }
        for (i, (b, label)) in self.buttons.iter().zip(&self.labels).enumerate() {
            let (look, enabled) = (self.states.look(i), self.states.enabled(i));
            draw_button(shapes, text, atlas, s, b.rect, label, look, enabled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::WEIGHTS;
    use super::*;

    #[test]
    fn buttons_stand_as_vanillas_do_under_the_logo() {
        let fonts = Fonts::new(&WEIGHTS);
        let p = PauseScreen::new(UiScale::new(1600, 900), &fonts);
        let actions: Vec<Action> = p.buttons.iter().map(|b| b.action).collect();
        assert_eq!(
            actions,
            [
                Action::Back,
                Action::Settings,
                Action::GameSettings,
                Action::Cancel
            ]
        );
        let r: Vec<Rect> = p.buttons.iter().map(|b| b.rect).collect();
        // The wide ones are as wide as the row of two, all of it in the middle.
        assert_eq!((r[0].x, r[0].w), (800.0 - 230.0, 460.0));
        assert_eq!((r[3].x, r[3].w), (r[0].x, r[0].w));
        assert_eq!((r[1].y, r[1].w), (r[2].y, r[2].w));
        assert_eq!(r[1].x, r[0].x);
        assert_eq!(r[2].x - (r[1].x + r[1].w), GAP);
        assert_eq!(r[2].x + r[2].w, r[0].x + r[0].w);
        assert!(r[0].y < r[1].y && r[1].y < r[3].y);
        // The gap between the two is in the very middle.
        let gap = 0.5 * (r[1].x + r[1].w + r[2].x);
        assert_eq!((gap, r[1].y + 0.5 * r[1].h), (800.0, 450.0));
        assert_eq!(p.logo.x + 0.5 * p.logo.w, 800.0);
        assert_eq!((p.logo.w, r[0].y - (p.logo.y + p.logo.h)), (LOGO, SPACE));
        assert!(p.logo.y + p.logo.h < r[0].y);
        assert_eq!(p.hit(800.0, r[0].y + 1.0), Some(0));
        assert_eq!(p.hit(r[2].x + 1.0, r[2].y + 1.0), Some(2));
        // Between the two of the row there is nothing.
        assert_eq!(p.hit(r[2].x - 1.0, r[2].y + 1.0), None);
        // Every label fits its pill, at every menu size.
        for factor in [0.9, 1.0, 1.3] {
            let s = UiScale::sized(1600, 900, factor);
            let p = PauseScreen::new(s, &fonts);
            for (b, l) in p.buttons.iter().zip(&p.labels) {
                assert!(l.width(&s) < b.rect.w - 40.0, "{}", b.label);
            }
        }
        // It stays in a small window.
        let small = PauseScreen::new(UiScale::new(854, 480), &fonts);
        let last = small.buttons[3].rect;
        assert!(last.y + last.h < small.scale.height);
        for factor in [1.0, 1.3] {
            let p = PauseScreen::new(UiScale::sized(640, 360, factor), &fonts);
            assert!(p.logo.y >= 0.0 && p.logo.w <= LOGO);
        }
    }

    #[test]
    fn laying_it_out_again_keeps_the_focus() {
        let fonts = Fonts::new(&WEIGHTS);
        let mut p = PauseScreen::new(UiScale::new(1600, 900), &fonts);
        p.states.set_focus(Some(2), true);
        p.layout(UiScale::new(1280, 720), &fonts);
        assert_eq!(p.states.focused(), Some(2));
    }
}
