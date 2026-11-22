//! Buttons (`client/gui/components/AbstractButton.java`, `WidgetSprites.java`,
//! `SpriteIconButton.java`). Labels come with the font.

use ruthenium_assets::Id;

use crate::draw::{DrawList, Sprites};

/// What a button shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// A translated label (its translation key).
    Label(&'static str),
    /// An icon sprite centred on the button, `width` x `height`.
    Icon { sprite: Id, width: i32, height: i32 },
}

/// A clickable button in GUI units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub content: Content,
    pub active: bool,
    /// What the screen does when it is pressed.
    pub action: Action,
}

/// What pressing a button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Multiplayer,
    Language,
    Accessibility,
    Options,
    Quit,
}

/// The three background sprites of a button.
#[derive(Debug, Clone)]
pub struct ButtonSprites {
    normal: Id,
    disabled: Id,
    highlighted: Id,
}

impl Default for ButtonSprites {
    fn default() -> ButtonSprites {
        ButtonSprites {
            normal: Id::minecraft("widget/button").unwrap(),
            disabled: Id::minecraft("widget/button_disabled").unwrap(),
            highlighted: Id::minecraft("widget/button_highlighted").unwrap(),
        }
    }
}

impl ButtonSprites {
    /// Active buttons light up when hovered or focused; inactive ones always show the
    /// disabled sprite.
    pub fn get(&self, active: bool, hovered: bool) -> &Id {
        match (active, hovered) {
            (true, true) => &self.highlighted,
            (true, false) => &self.normal,
            (false, _) => &self.disabled,
        }
    }
}

impl Button {
    pub fn new(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        content: Content,
        action: Action,
    ) -> Button {
        Button {
            x,
            y,
            width,
            height,
            content,
            active: true,
            action,
        }
    }

    /// Whether a point in GUI units is over the button (`AbstractWidget.isMouseOver`).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && y >= f64::from(self.y)
            && x < f64::from(self.x + self.width)
            && y < f64::from(self.y + self.height)
    }

    /// Draws the background and, for icon buttons, the centred icon.
    pub fn draw(
        &self,
        dl: &mut DrawList,
        sprites: &Sprites,
        look: &ButtonSprites,
        mouse: Option<(f64, f64)>,
        alpha: f32,
    ) {
        let hovered = mouse.is_some_and(|(x, y)| self.contains(x, y));
        let color = white(alpha);
        dl.blit_sprite(
            sprites,
            look.get(self.active, hovered),
            self.x,
            self.y,
            self.width,
            self.height,
            color,
        );
        if let Content::Icon {
            sprite,
            width,
            height,
        } = &self.content
        {
            let x = self.x + self.width / 2 - width / 2;
            let y = self.y + self.height / 2 - height / 2;
            dl.blit_sprite(sprites, sprite, x, y, *width, *height, color);
        }
    }
}

/// `ARGB.white(alpha)`: white with `floor(alpha * 255)` alpha.
pub fn white(alpha: f32) -> u32 {
    (((alpha * 255.0).floor() as i32 as u32) & 0xFF) << 24 | 0x00FF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::tests::sheet;
    use ruthenium_assets::meta::GuiScaling;

    #[test]
    fn hover_bounds_are_half_open() {
        let b = Button::new(10, 10, 20, 20, Content::Label("x"), Action::Quit);
        assert!(b.contains(10.0, 10.0));
        assert!(b.contains(29.9, 29.9));
        assert!(!b.contains(30.0, 15.0));
        assert!(!b.contains(9.9, 15.0));
    }

    #[test]
    fn sprite_choice() {
        let s = ButtonSprites::default();
        assert_eq!(s.get(true, false).path(), "widget/button");
        assert_eq!(s.get(true, true).path(), "widget/button_highlighted");
        assert_eq!(s.get(false, true).path(), "widget/button_disabled");
        assert_eq!(white(1.0), 0xFFFF_FFFF);
        assert_eq!(white(0.5), 0x7FFF_FFFF);
    }

    #[test]
    fn icons_are_centred() {
        let sheet = sheet(GuiScaling::Stretch, 16, 16);
        let icon = Content::Icon {
            sprite: Id::minecraft("icon/language").unwrap(),
            width: 15,
            height: 15,
        };
        let b = Button::new(100, 50, 20, 20, icon, Action::Language);
        let mut dl = DrawList::new();
        b.draw(&mut dl, &sheet, &ButtonSprites::default(), None, 1.0);
        assert_eq!(dl.quads().len(), 2);
        // 100 + 10 - 7 = 103.
        assert_eq!(dl.quads()[1].rect, [103.0, 53.0, 118.0, 68.0]);
    }
}
