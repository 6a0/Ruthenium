//! The player's mouse and keys as the game reads them: the mouse's turn
//! (`MouseHandler.onMove`, `handleAccumulatedMovement`, `turnPlayer`), the scroll
//! wheel (`ScrollWheelHandler`) and the movement keys (`KeyboardInput`), under the
//! player's bindings ([`crate::keybinds`]).
//!
//! Turning is exact: the relative motion SDL reports (floats) is summed in doubles over
//! a frame, turned into `Entity.turn`'s amounts once per frame with vanilla's
//! sensitivity curve in doubles, and `turn` itself works in floats
//! ([`crate::player::LocalPlayer::turn`]). Every rotation the server sees is then one
//! vanilla could have sent with the same sensitivity and mouse.

use crate::player::Keys;

/// The mouse options turning reads (`Options.sensitivity`, `invertXMouse`,
/// `invertYMouse`). The sensitivity is any double from 0 to 1, as vanilla's slider
/// stores it (its value comes from the pointer's position, not from steps), so it's
/// best imported from vanilla's `options.txt` as it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseOptions {
    pub sensitivity: f64,
    pub invert_x: bool,
    pub invert_y: bool,
}

impl MouseOptions {
    /// Vanilla's defaults.
    pub const DEFAULT: MouseOptions = MouseOptions {
        sensitivity: 0.5,
        invert_x: false,
        invert_y: false,
    };
}

impl Default for MouseOptions {
    fn default() -> MouseOptions {
        MouseOptions::DEFAULT
    }
}

/// `MouseHandler.turnPlayer` without the smooth camera or a spyglass: the amounts handed
/// to `Entity.turn` for the motion `(dx, dy)` gathered over one frame.
pub fn turn_amount(dx: f64, dy: f64, o: &MouseOptions) -> (f64, f64) {
    // The float constants 0.6F and 0.2F widened to doubles, as the game writes them.
    let ss = o.sensitivity * 0.6000000238418579 + 0.20000000298023224;
    let sensitivity_mod = ss * ss * ss;
    let sens = sensitivity_mod * 8.0;
    let xo = dx * sens;
    let yo = dy * sens;
    (
        if o.invert_x { -xo } else { xo },
        if o.invert_y { -yo } else { yo },
    )
}

/// The mouse's relative motion summed over a frame while it's grabbed
/// (`MouseHandler.onMove`): the first motion after a grab is dropped
/// (`ignoreFirstMove`, it's the warp to the window's centre), and motion only counts
/// while the window is active.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MouseMotion {
    dx: f64,
    dy: f64,
    ignore_first_move: bool,
}

impl MouseMotion {
    pub fn new() -> MouseMotion {
        MouseMotion {
            dx: 0.0,
            dy: 0.0,
            ignore_first_move: true,
        }
    }

    /// The mouse was grabbed, or entered the window (`grabMouse`, `cursorEntered`).
    pub fn ignore_next(&mut self) {
        self.ignore_first_move = true;
    }

    /// One motion event: SDL's `xrel`, `yrel`.
    pub fn moved(&mut self, xrel: f32, yrel: f32, window_active: bool) {
        if self.ignore_first_move {
            self.ignore_first_move = false;
        } else if window_active {
            self.dx += f64::from(xrel);
            self.dy += f64::from(yrel);
        }
    }

    /// The frame's motion, cleared (`handleAccumulatedMovement`).
    pub fn take(&mut self) -> (f64, f64) {
        let d = (self.dx, self.dy);
        self.dx = 0.0;
        self.dy = 0.0;
        d
    }
}

/// Java's `Math.signum`: zero (of either sign) and NaN are returned as they are.
pub fn signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() {
        v
    } else if v > 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Wheel motion gathered into whole notches (`ScrollWheelHandler`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScrollWheel {
    x: f64,
    y: f64,
}

impl ScrollWheel {
    /// One wheel event (`MouseHandler.onScroll` with vanilla's default wheel options:
    /// no discrete scrolling, sensitivity 1): the notches to scroll the hotbar by, if
    /// any (`wheelY`, or `-wheelX` without vertical motion).
    pub fn scrolled(&mut self, x: f64, y: f64) -> Option<i32> {
        let (x, y) = (x * 1.0, y * 1.0);
        if self.x != 0.0 && signum(x) != signum(self.x) {
            self.x = 0.0;
        }
        if self.y != 0.0 && signum(y) != signum(self.y) {
            self.y = 0.0;
        }
        self.x += x;
        self.y += y;
        let wheel_x = self.x as i32;
        let wheel_y = self.y as i32;
        if wheel_x == 0 && wheel_y == 0 {
            return None;
        }
        self.x -= f64::from(wheel_x);
        self.y -= f64::from(wheel_y);
        Some(if wheel_y == 0 { -wheel_x } else { wheel_y })
    }
}

/// `ScrollWheelHandler.getNextScrollWheelSelection`.
pub fn next_scroll_selection(wheel: f64, current: i32, limit: i32) -> i32 {
    let step = signum(wheel) as i32;
    let mut selected = (current - step).max(-1);
    while selected < 0 {
        selected += limit;
    }
    while selected >= limit {
        selected -= limit;
    }
    selected
}

use crate::keybinds::Action;
pub use crate::keybinds::{Key, KeyBinds};

/// Which keys and buttons are down (`KeyMapping.set` from key and button events while no
/// screen is open; `releaseAll` when one opens), read as movement input
/// (`KeyboardInput.tick`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeldKeys {
    down: Vec<Key>,
}

impl HeldKeys {
    pub fn set(&mut self, key: Key, down: bool) {
        let at = self.down.iter().position(|&k| k == key);
        match (down, at) {
            (true, None) => self.down.push(key),
            (false, Some(i)) => {
                self.down.swap_remove(i);
            }
            _ => {}
        }
    }

    pub fn release_all(&mut self) {
        self.down.clear();
    }

    pub fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    /// Whether a mapping is held: its key is down (an unbound one never is).
    pub fn action_down(&self, binds: &KeyBinds, action: Action) -> bool {
        let key = binds.get(action);
        !key.is_unbound() && self.is_down(key)
    }

    /// The keyboard keys held.
    pub fn keyboard(&self) -> impl Iterator<Item = Key> + '_ {
        self.down
            .iter()
            .copied()
            .filter(|k| matches!(k, Key::Keyboard(_)))
    }

    /// The movement keys held, under `binds` (toggle sneak and sprint off, vanilla's
    /// default).
    pub fn keys(&self, binds: &KeyBinds) -> Keys {
        let down = |a| self.action_down(binds, a);
        Keys {
            forward: down(Action::Forward),
            backward: down(Action::Back),
            left: down(Action::Left),
            right: down(Action::Right),
            jump: down(Action::Jump),
            shift: down(Action::Sneak),
            sprint: down(Action::Sprint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sensitivity_turns_about_one_to_one() {
        // 0.5 gives ss = 0.5000000149011612, sens = 8 ss^3, a hair over 1.
        let (xo, yo) = turn_amount(25.0, -4.0, &MouseOptions::DEFAULT);
        let ss = 0.5 * 0.6000000238418579 + 0.20000000298023224;
        assert_eq!(xo, 25.0 * (ss * ss * ss * 8.0));
        assert_eq!(yo, -4.0 * (ss * ss * ss * 8.0));
        assert!(xo > 25.0 && xo < 25.00001);
        let inverted = MouseOptions {
            invert_y: true,
            ..MouseOptions::DEFAULT
        };
        assert_eq!(
            turn_amount(1.0, 2.0, &inverted).1,
            -turn_amount(1.0, 2.0, &MouseOptions::DEFAULT).1
        );
        // Sensitivity 0 still turns (ss = 0.2): vanilla's "Yawn" isn't zero.
        let zero = MouseOptions {
            sensitivity: 0.0,
            ..MouseOptions::DEFAULT
        };
        assert_eq!(turn_amount(1.0, 0.0, &zero).0, {
            let ss = 0.20000000298023224f64;
            ss * ss * ss * 8.0
        });
    }

    #[test]
    fn motion_skips_the_first_move_and_sums_in_doubles() {
        let mut m = MouseMotion::new();
        m.moved(100.0, 100.0, true);
        assert_eq!(m.take(), (0.0, 0.0));
        m.moved(0.1, -0.3, true);
        m.moved(0.2, 0.0, true);
        m.moved(5.0, 5.0, false);
        assert_eq!(
            m.take(),
            (f64::from(0.1f32) + f64::from(0.2f32), f64::from(-0.3f32))
        );
        assert_eq!(m.take(), (0.0, 0.0));
    }

    #[test]
    fn scroll_wheel_gathers_notches_like_vanilla() {
        let mut w = ScrollWheel::default();
        assert_eq!(w.scrolled(0.0, 0.5), None);
        assert_eq!(w.scrolled(0.0, 0.5), Some(1));
        // A turn the other way drops what was gathered.
        assert_eq!(w.scrolled(0.0, 0.75), None);
        assert_eq!(w.scrolled(0.0, -0.5), None);
        assert_eq!(w.scrolled(0.0, -0.5), Some(-1));
        assert_eq!(w.scrolled(2.0, 0.0), Some(-2));
        // Scrolling up (positive) selects the slot to the left, with wrap.
        assert_eq!(next_scroll_selection(1.0, 0, 9), 8);
        assert_eq!(next_scroll_selection(-1.0, 8, 9), 0);
        assert_eq!(next_scroll_selection(-3.0, 4, 9), 5);
        assert_eq!(next_scroll_selection(0.0, 4, 9), 4);
        assert_eq!(signum(-0.0).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn held_keys_read_as_movement() {
        let mut b = KeyBinds::DEFAULT;
        let mut h = HeldKeys::default();
        h.set(b.get(Action::Forward), true);
        h.set(b.get(Action::Sprint), true);
        h.set(b.get(Action::Forward), true);
        assert_eq!(
            h.keys(&b),
            Keys {
                forward: true,
                sprint: true,
                ..Keys::default()
            }
        );
        h.set(b.get(Action::Forward), false);
        assert!(!h.keys(&b).forward);
        h.release_all();
        assert_eq!(h.keys(&b), Keys::default());
        // Rebound: a mouse button walks, the old key does nothing, and nothing holds
        // an unbound mapping.
        b.set(Action::Forward, Key::Mouse(4));
        b.set(Action::Jump, Key::UNBOUND);
        h.set(Key::Keyboard(26), true);
        h.set(Key::UNBOUND, true);
        assert_eq!(h.keys(&b), Keys::default());
        h.set(Key::Mouse(4), true);
        assert!(h.keys(&b).forward && !h.keys(&b).jump);
        assert_eq!(
            h.keyboard().collect::<Vec<_>>(),
            [Key::Keyboard(26), Key::UNBOUND]
        );
    }
}
