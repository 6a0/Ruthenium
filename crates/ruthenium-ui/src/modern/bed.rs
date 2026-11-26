//! The bed screen's Leave Bed (`InBedChatScreen`): one glass pill in the middle at the
//! bottom of the window, its lower edge where vanilla's button has its own, over the
//! chat, which the HUD draws in vanilla's look.

use super::{
    Action, Button, ButtonStates, LABEL_SIZE, Rect, ShapeList, Text, UiScale, draw_button,
};
use crate::scale::GuiScale;
use crate::text::{Fonts, GlyphAtlas, TextList, Weight};

/// The pill: the title screen's buttons' size (vanilla's are 200 GUI units wide there and
/// here).
const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 52.0;
/// From the bottom of the GUI to the bottom of vanilla's button, in GUI units (it is 20
/// high at `height - 40`).
const VANILLA_BOTTOM: u32 = 20;

/// See the module docs.
#[derive(Debug, Clone)]
pub struct BedScreen {
    pub scale: UiScale,
    pub states: ButtonStates,
    pub button: Button,
    label: Text,
}

impl BedScreen {
    pub fn new(scale: UiScale, fonts: &Fonts) -> BedScreen {
        let button = Button {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: WIDTH,
                h: HEIGHT,
            },
            label: "Leave Bed",
            action: Action::Confirm,
        };
        let label = Text::new(
            fonts,
            button.label,
            &scale,
            LABEL_SIZE,
            Weight::SEMIBOLD,
            0.01,
        );
        let mut screen = BedScreen {
            scale,
            states: ButtonStates::new(1),
            button,
            label,
        };
        screen.place(None);
        screen
    }

    /// Lays the screen out for `scale`, keeping the pill's state; `gui` is the HUD's
    /// scale ([`Self::place`]).
    pub fn layout(&mut self, scale: UiScale, fonts: &Fonts, gui: Option<GuiScale>) {
        self.scale = scale;
        self.label = Text::new(
            fonts,
            self.button.label,
            &scale,
            LABEL_SIZE,
            Weight::SEMIBOLD,
            0.01,
        );
        self.place(gui);
    }

    /// Puts the pill's lower edge where vanilla's button has its own at the HUD's scale
    /// `gui` (none: its own height over the bottom), in the middle across.
    pub fn place(&mut self, gui: Option<GuiScale>) {
        let s = &self.scale;
        let bottom = match gui {
            Some(g) => {
                let px = g.height.saturating_sub(VANILLA_BOTTOM) * g.scale;
                px as f32 / s.px
            }
            None => s.height - HEIGHT,
        };
        self.button.rect.x = 0.5 * (s.width - WIDTH);
        self.button.rect.y = (bottom - HEIGHT).max(0.0);
    }

    /// Whether `(x, y)` (UI units) is on the pill.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.button.rect.contains(x, y).then_some(0)
    }

    /// Adds the pill's shapes and, once the glyphs are on the GPU (`atlas`), its label.
    pub fn draw(&self, shapes: &mut ShapeList, text: &mut TextList, atlas: Option<&GlyphAtlas>) {
        let look = self.states.look(0);
        let enabled = self.states.enabled(0);
        draw_button(
            shapes,
            text,
            atlas,
            &self.scale,
            self.button.rect,
            &self.label,
            look,
            enabled,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::WEIGHTS;
    use super::*;

    #[test]
    fn leave_bed_stands_where_vanillas_button_does() {
        let fonts = Fonts::new(&WEIGHTS);
        // 1920 by 1080 at GUI scale 4: the GUI is 480 by 270, vanilla's button from 230
        // to 250 down, so its lower edge 1000 pixels down; a UI unit is 1.2 pixels.
        let scale = UiScale::new(1920, 1080);
        let mut b = BedScreen::new(scale, &fonts);
        b.place(Some(GuiScale::new(1920, 1080, 0, false)));
        let r = b.button.rect;
        assert!((r.x + 0.5 * r.w - 800.0).abs() < 1e-3, "{r:?}");
        assert_eq!((r.w, r.h), (WIDTH, HEIGHT));
        assert!((r.y + r.h - 1000.0 / 1.2).abs() < 1e-3, "{r:?}");
        assert_eq!(b.hit(800.0, r.y + 1.0), Some(0));
        assert_eq!(b.hit(800.0, r.y - 1.0), None);
        // GUI scale 1 at an odd height: 20 units over the bottom of the GUI's 1081.
        let scale = UiScale::new(1280, 1081);
        b.layout(scale, &fonts, Some(GuiScale::new(1280, 1081, 1, false)));
        let r = b.button.rect;
        assert!((r.y + r.h - 1061.0 / scale.px).abs() < 1e-3, "{r:?}");
        assert!((r.x + 0.5 * r.w - 0.5 * scale.width).abs() < 1e-3, "{r:?}");
        // Without the HUD's scale: its own height over the bottom. A window too low: at
        // the top.
        b.place(None);
        assert_eq!(b.button.rect.y + HEIGHT, scale.height - HEIGHT);
        let scale = UiScale::new(320, 40);
        b.layout(scale, &fonts, Some(GuiScale::new(320, 40, 0, false)));
        assert_eq!(b.button.rect.y, 0.0);
        // A GUI lower than vanilla's button stands off the bottom.
        b.layout(
            UiScale::new(320, 10),
            &fonts,
            Some(GuiScale::new(320, 10, 0, false)),
        );
        assert_eq!(b.button.rect.y, 0.0);
        assert_eq!(
            (b.button.label, b.button.action),
            ("Leave Bed", Action::Confirm)
        );
    }
}
