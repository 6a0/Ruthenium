//! Ruthenium's own menus (title, server list, accounts, settings, connecting): a modern,
//! space-themed look, unrelated to vanilla's GUI. Layout is in UI units, scaled to the
//! window by [`UiScale`]; shapes come out in window pixels.

/// Window pixels per UI unit: the layout is designed for a window 900 units tall, so it
/// keeps its proportions at any size (never below 0.75 px per unit, so text stays
/// readable in small windows).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiScale {
    pub px: f32,
    /// Window size in UI units.
    pub width: f32,
    pub height: f32,
}

impl UiScale {
    const DESIGN_HEIGHT: f32 = 900.0;
    const MIN: f32 = 0.75;

    pub fn new(pixel_width: u32, pixel_height: u32) -> UiScale {
        let px = (pixel_height as f32 / Self::DESIGN_HEIGHT).max(Self::MIN);
        UiScale {
            px,
            width: pixel_width as f32 / px,
            height: pixel_height as f32 / px,
        }
    }

    /// Window coordinates (points) to UI units, for a window `window_size` points large
    /// and `pixel_size` pixels large.
    pub fn from_window(
        &self,
        x: f32,
        y: f32,
        window_size: (u32, u32),
        pixel_size: (u32, u32),
    ) -> (f32, f32) {
        let sx = pixel_size.0 as f32 / window_size.0.max(1) as f32;
        let sy = pixel_size.1 as f32 / window_size.1.max(1) as f32;
        (x * sx / self.px, y * sy / self.px)
    }
}

/// A rectangle in UI units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// One rounded shape for the renderer's shape pipeline (`shape.vert/frag`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// `x0, y0, x1, y1` in window pixels.
    pub rect: [f32; 4],
    /// Straight-alpha RGBA: the fill, or for glass the tint over the backdrop.
    pub fill: [f32; 4],
    /// Corner radius in pixels (half the height makes a pill).
    pub radius: f32,
    /// See-through frosted glass instead of a flat fill.
    pub glass: bool,
    /// Strength of the glass rim light (0 to 1).
    pub highlight: f32,
    /// How far, in pixels, the glass bends the backdrop at its rim.
    pub refraction: f32,
}

/// Shapes for one frame, back to front.
#[derive(Debug, Default)]
pub struct ShapeList {
    shapes: Vec<Shape>,
}

impl ShapeList {
    pub fn new() -> ShapeList {
        ShapeList {
            shapes: Vec::with_capacity(64),
        }
    }

    pub fn clear(&mut self) {
        self.shapes.clear();
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    pub fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
    }

    /// A glass pill over `r` (UI units).
    pub fn glass_pill(&mut self, scale: &UiScale, r: Rect, tint: [f32; 4], highlight: f32) {
        let px = scale.px;
        let rect = [r.x * px, r.y * px, (r.x + r.w) * px, (r.y + r.h) * px];
        self.push(Shape {
            rect,
            fill: tint,
            radius: 0.5 * r.h * px,
            glass: true,
            highlight,
            refraction: 0.22 * r.h * px,
        });
    }
}

/// What a menu button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Multiplayer,
    Settings,
    Quit,
}

/// A pill button.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub rect: Rect,
    /// The label (Ruthenium's own text; translations come later).
    pub label: &'static str,
    pub action: Action,
}

/// Glass tint at rest and under the pointer.
const TINT: [f32; 4] = [0.85, 0.88, 1.0, 0.04];
const TINT_HOVER: [f32; 4] = [0.90, 0.93, 1.0, 0.12];

/// The title screen: a wordmark (reserved space for now) above a centred column of
/// glass pills.
#[derive(Debug, Clone)]
pub struct TitleScreen {
    pub scale: UiScale,
    pub wordmark: Rect,
    pub buttons: Vec<Button>,
}

impl TitleScreen {
    pub const BUTTON_WIDTH: f32 = 300.0;
    pub const BUTTON_HEIGHT: f32 = 52.0;
    pub const GAP: f32 = 14.0;

    pub fn new(scale: UiScale) -> TitleScreen {
        let entries = [
            ("Multiplayer", Action::Multiplayer),
            ("Settings", Action::Settings),
            ("Quit", Action::Quit),
        ];
        let n = entries.len() as f32;
        let column = n * Self::BUTTON_HEIGHT + (n - 1.0) * Self::GAP;
        let wordmark_h = 120.0;
        let space = 56.0;
        // The wordmark and the column together sit a little above the middle.
        let top = ((scale.height - (wordmark_h + space + column)) * 0.45).max(24.0);
        let cx = scale.width * 0.5;
        let wordmark = Rect {
            x: cx - 260.0,
            y: top,
            w: 520.0,
            h: wordmark_h,
        };
        let mut y = top + wordmark_h + space;
        let buttons = entries
            .into_iter()
            .map(|(label, action)| {
                let b = Button {
                    rect: Rect {
                        x: cx - Self::BUTTON_WIDTH * 0.5,
                        y,
                        w: Self::BUTTON_WIDTH,
                        h: Self::BUTTON_HEIGHT,
                    },
                    label,
                    action,
                };
                y += Self::BUTTON_HEIGHT + Self::GAP;
                b
            })
            .collect();
        TitleScreen {
            scale,
            wordmark,
            buttons,
        }
    }

    /// Adds the screen's shapes; `mouse` is in UI units.
    pub fn draw(&self, shapes: &mut ShapeList, mouse: Option<(f32, f32)>) {
        for b in &self.buttons {
            let hovered = mouse.is_some_and(|(x, y)| b.rect.contains(x, y));
            let (tint, light) = if hovered {
                (TINT_HOVER, 1.0)
            } else {
                (TINT, 0.75)
            };
            shapes.glass_pill(&self.scale, b.rect, tint, light);
        }
    }

    /// The action of the button under a click at `(x, y)` (UI units).
    pub fn click(&self, x: f32, y: f32) -> Option<Action> {
        self.buttons
            .iter()
            .find(|b| b.rect.contains(x, y))
            .map(|b| b.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn scale_follows_window_height() {
        let s = UiScale::new(1920, 1080);
        assert_eq!(s.px, 1.2);
        assert!(near((s.width, s.height), (1600.0, 900.0)));
        // Small windows bottom out at 0.75 px per unit.
        assert_eq!(UiScale::new(640, 360).px, 0.75);
    }

    #[test]
    fn title_column_is_centred_and_clickable() {
        let s = UiScale::new(1600, 900);
        let t = TitleScreen::new(s);
        assert_eq!(t.buttons.len(), 3);
        for b in &t.buttons {
            assert_eq!(b.rect.x + b.rect.w * 0.5, 800.0);
        }
        let first = t.buttons[0].rect;
        assert!(t.wordmark.y + t.wordmark.h < first.y);
        assert_eq!(t.click(800.0, first.y + 1.0), Some(Action::Multiplayer));
        let last = t.buttons[2].rect;
        assert_eq!(t.click(800.0, last.y + last.h - 0.5), Some(Action::Quit));
        assert_eq!(t.click(10.0, 10.0), None);
        // Pills: radius is half the height, in pixels.
        let mut shapes = ShapeList::new();
        t.draw(&mut shapes, None);
        let p = shapes.shapes()[0];
        assert_eq!(p.radius, 0.5 * (p.rect[3] - p.rect[1]));
        assert!(p.glass);
    }

    #[test]
    fn window_points_to_ui_units() {
        let s = UiScale::new(1920, 1080);
        // HiDPI: 960 x 540 points over 1920 x 1080 pixels.
        assert!(near(
            s.from_window(480.0, 270.0, (960, 540), (1920, 1080)),
            (800.0, 450.0)
        ));
    }
}
