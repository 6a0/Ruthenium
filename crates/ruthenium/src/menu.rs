//! Ruthenium's out-of-game menus (for now the title screen): lays them out for the
//! window and builds each frame's shapes.

use ruthenium_render::GuiFrame;
use ruthenium_ui::modern::{Action, ShapeList, TitleScreen, UiScale};

/// See the module docs.
#[derive(Debug)]
pub struct Menu {
    title: Option<TitleScreen>,
    shapes: ShapeList,
    /// Mouse position in window coordinates.
    mouse: Option<(f32, f32)>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu {
            title: None,
            shapes: ShapeList::new(),
            mouse: None,
        }
    }

    pub fn mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse = Some((x, y));
    }

    fn layout(&mut self, pixel_size: (u32, u32)) -> &TitleScreen {
        let scale = UiScale::new(pixel_size.0, pixel_size.1);
        if self.title.as_ref().is_none_or(|t| t.scale != scale) {
            self.title = Some(TitleScreen::new(scale));
        }
        self.title.as_ref().unwrap()
    }

    /// The action of a left click at `(x, y)` (window coordinates), if it hit a button.
    pub fn click(
        &mut self,
        x: f32,
        y: f32,
        pixel_size: (u32, u32),
        window_size: (u32, u32),
    ) -> Option<Action> {
        let title = self.layout(pixel_size);
        let (ux, uy) = title.scale.from_window(x, y, window_size, pixel_size);
        title.click(ux, uy)
    }

    /// This frame's GUI.
    pub fn frame(&mut self, pixel_size: (u32, u32), window_size: (u32, u32)) -> GuiFrame<'_> {
        let mouse = self.mouse;
        self.layout(pixel_size);
        let title = self.title.as_ref().unwrap();
        let mouse = mouse.map(|(x, y)| title.scale.from_window(x, y, window_size, pixel_size));
        self.shapes.clear();
        title.draw(&mut self.shapes, mouse);
        GuiFrame {
            shapes: self.shapes.shapes(),
            ..GuiFrame::default()
        }
    }
}
