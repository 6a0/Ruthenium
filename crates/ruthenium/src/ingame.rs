//! In the world: the mouse grabbed and turning the camera, the movement and hotbar keys
//! and the scroll wheel handed to the game thread, and, until the world is drawn, a
//! readout of where the player is.
//!
//! Vanilla's rules: keys count only while no screen is open (`KeyMapping.releaseAll`
//! when one opens); the mouse's motion is summed over a frame and turns the camera once
//! per frame, after the frame's events (`handleAccumulatedMovement`), the first motion
//! after a grab dropped; the game pauses (opens the Game Menu) once the window has been
//! out of focus for half a second (`pauseIfInactive`, `pauseOnLostFocus` on).

use std::time::{Duration, Instant};

use ruthenium_sim::input::{self, HeldKeys, KeyBinds, MouseMotion, MouseOptions, ScrollWheel};
use ruthenium_sim::player::Pose;
use ruthenium_sim::session::{PlayerView, Session};
use ruthenium_ui::modern::{Rect, ShapeList, UiScale};
use ruthenium_ui::text::{Fonts, GlyphAtlas, TextList, TextStyle, Weight};

/// How long the window may be out of focus before the game pauses.
const PAUSE_AFTER: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct InGame {
    motion: MouseMotion,
    held: HeldKeys,
    wheel: ScrollWheel,
    binds: KeyBinds,
    focused: bool,
    /// Since when the window has been out of focus.
    unfocused_since: Option<Instant>,
}

impl Default for InGame {
    fn default() -> InGame {
        InGame {
            motion: MouseMotion::new(),
            held: HeldKeys::default(),
            wheel: ScrollWheel::default(),
            binds: KeyBinds::DEFAULT,
            focused: true,
            unfocused_since: None,
        }
    }
}

impl InGame {
    /// The screen closed and the mouse is grabbed again (`grabMouse`).
    pub fn enter(&mut self, session: Option<&Session>) {
        self.motion = MouseMotion::new();
        self.release(session);
    }

    /// A screen opened (`KeyMapping.releaseAll`).
    pub fn release(&mut self, session: Option<&Session>) {
        self.held.release_all();
        if let Some(s) = session {
            s.set_keys(Default::default());
        }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn set_focus(&mut self, on: bool, session: Option<&Session>) {
        self.focused = on;
        self.unfocused_since = if on { None } else { Some(Instant::now()) };
        if !on {
            self.release(session);
        }
    }

    /// Whether the game should pause: out of focus long enough.
    pub fn should_pause(&self) -> bool {
        self.unfocused_since
            .is_some_and(|t| t.elapsed() > PAUSE_AFTER)
    }

    /// A key went down or up while in the world (not Escape).
    pub fn key(&mut self, scancode: u32, down: bool, session: Option<&Session>) {
        self.held.set(scancode, down);
        // `KeyMapping.click`: every press, repeats too, as vanilla counts them.
        if down
            && let Some(slot) = self.binds.hotbar_slot(scancode)
            && let Some(s) = session
        {
            s.hotbar_key(slot);
        }
        if let Some(s) = session {
            s.set_keys(self.held.keys(&self.binds));
        }
    }

    /// Relative mouse motion (SDL's `xrel`, `yrel`).
    pub fn mouse_moved(&mut self, dx: f32, dy: f32) {
        self.motion.moved(dx, dy, self.focused);
    }

    /// The wheel, as SDL reports it (no natural-scrolling flip, as vanilla reads it).
    pub fn wheel(&mut self, x: f32, y: f32, session: Option<&Session>) {
        if let Some(w) = self.wheel.scrolled(f64::from(x), f64::from(y))
            && let Some(s) = session
        {
            s.scroll(w);
        }
    }

    /// The frame's turn of the camera, after its events.
    pub fn frame(&mut self, mouse: &MouseOptions, session: Option<&Session>) {
        let (dx, dy) = self.motion.take();
        if !self.focused || (dx == 0.0 && dy == 0.0) {
            return;
        }
        let (xo, yo) = input::turn_amount(dx, dy, mouse);
        if let Some(s) = session {
            s.turn(xo, yo);
        }
    }
}

/// `Mth.wrapDegrees` for floats.
fn wrap_degrees(v: f32) -> f32 {
    let mut w = v % 360.0;
    if w >= 180.0 {
        w -= 360.0;
    }
    if w < -180.0 {
        w += 360.0;
    }
    w
}

/// The readout: where the player is and what it's doing, a crosshair, and how to get
/// to the menu.
pub fn draw_hud(
    view: Option<&PlayerView>,
    server: &str,
    scale: &UiScale,
    fonts: &Fonts,
    atlas: Option<&GlyphAtlas>,
    shapes: &mut ShapeList,
    text: &mut TextList,
) {
    let (cx, cy) = (scale.width / 2.0, scale.height / 2.0);
    let white = [1.0, 1.0, 1.0, 0.9];
    shapes.rounded(
        scale,
        Rect {
            x: cx - 9.0,
            y: cy - 1.0,
            w: 18.0,
            h: 2.0,
        },
        1.0,
        white,
    );
    shapes.rounded(
        scale,
        Rect {
            x: cx - 1.0,
            y: cy - 9.0,
            w: 2.0,
            h: 18.0,
        },
        1.0,
        white,
    );
    let Some(atlas) = atlas else { return };
    let style = |size: f32, weight| TextStyle {
        size: size * scale.px,
        weight,
        tracking: 0.0,
    };
    let mut lines = vec![(server.to_owned(), style(20.0, Weight::SEMIBOLD))];
    let body = style(16.0, Weight::MEDIUM);
    match view {
        Some(v) => {
            let p = v.position;
            lines.push((
                format!("XYZ: {:.3} / {:.5} / {:.3}", p[0], p[1], p[2]),
                body,
            ));
            lines.push((
                format!("Facing: {:.1} / {:.1}", wrap_degrees(v.y_rot), v.x_rot),
                body,
            ));
            let mut state = vec![format!("Slot {}", v.selected_slot + 1)];
            if v.on_ground {
                state.push("on the ground".into());
            }
            if v.sprinting {
                state.push("sprinting".into());
            }
            match v.pose {
                Pose::Crouching => state.push("crouching".into()),
                Pose::Swimming => state.push("swimming".into()),
                Pose::Standing => {}
            }
            lines.push((state.join(", "), body));
            lines.push((format!("{} ({} chunks)", v.dimension, v.chunks), body));
        }
        None => lines.push(("Joining...".into(), body)),
    }
    let mut y = 36.0 * scale.px;
    for (s, st) in &lines {
        let line = fonts.shape(s, *st);
        text.line(atlas, &line, 24.0 * scale.px, y, white);
        y += st.size * 1.5;
    }
    let hint = fonts.shape(
        "Ruthenium can't draw the world yet. Esc for the menu.",
        style(15.0, Weight::MEDIUM),
    );
    text.line(
        atlas,
        &hint,
        (scale.width * scale.px - hint.width) / 2.0,
        (scale.height - 32.0) * scale.px,
        [1.0, 1.0, 1.0, 0.6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_degrees_like_vanilla() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-180.0), -180.0);
        assert_eq!(wrap_degrees(180.0), -180.0);
        assert_eq!(wrap_degrees(-540.0), -180.0);
        assert_eq!(wrap_degrees(725.0), 5.0);
    }

    #[test]
    fn hotbar_and_movement_keys_reach_the_input() {
        let mut g = InGame::default();
        g.enter(None);
        g.key(KeyBinds::DEFAULT.forward, true, None);
        assert!(g.held.keys(&g.binds).forward);
        g.set_focus(false, None);
        assert!(
            !g.held.keys(&g.binds).forward,
            "focus loss lets go of the keys"
        );
        assert!(!g.should_pause(), "not before half a second");
        g.unfocused_since = Some(Instant::now() - Duration::from_secs(1));
        assert!(g.should_pause());
        g.set_focus(true, None);
        assert!(!g.should_pause());
    }
}
