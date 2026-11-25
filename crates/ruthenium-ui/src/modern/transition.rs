//! How the menu over the world comes and goes: quickly, with nothing moving. The world
//! loses colour, darkens a little and blurs a little; the buttons come out of a heavy blur
//! and go back into one.

/// Seconds the world takes to change, and to be as it was.
const VEIL_IN: f32 = 0.18;
const VEIL_OUT: f32 = 0.15;
/// Seconds the buttons take to come, and to go.
const MENU_IN: f32 = 0.17;
const MENU_OUT: f32 = 0.11;
/// The share of their time after which the buttons are opaque.
const OPAQUE_AT: f32 = 0.7;

/// What is left of the world's colour and of its brightness behind the menu, and how far
/// it is blurred (UI units).
const SATURATION: f32 = 0.3;
const BRIGHTNESS: f32 = 0.78;
/// Its brightness behind the options, which have small print to be read.
const BRIGHTNESS_OPTIONS: f32 = 0.6;
const BLUR: f32 = 2.6;
/// How far the buttons are blurred at the first (UI units).
const MENU_BLUR: f32 = 16.0;

/// Slow at both ends (Hermite's smoothstep).
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// See the module docs: how far the world's change and the buttons have come, each a
/// share of its time from 0 to 1. Turned round half way, both go back from where they are.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MenuTransition {
    veil: f32,
    menu: f32,
    /// How far from the menu's brightness to the options'.
    options: f32,
}

/// How a frame draws it: lengths in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuLook {
    pub saturation: f32,
    pub brightness: f32,
    pub blur: f32,
    pub menu_blur: f32,
    pub menu_opacity: f32,
}

impl MenuTransition {
    /// So far come, the world's change and the buttons alike.
    pub fn at(share: f32) -> MenuTransition {
        let share = share.clamp(0.0, 1.0);
        MenuTransition {
            veil: share,
            menu: share,
            options: 0.0,
        }
    }

    /// Advances it by `dt` seconds towards open or closed, with the menu or its
    /// `options`.
    pub fn step(&mut self, open: bool, options: bool, dt: f32) {
        let towards = |v: &mut f32, on: bool, rise: f32, fall: f32| {
            *v = if on {
                (*v + dt / rise).min(1.0)
            } else {
                (*v - dt / fall).max(0.0)
            };
        };
        towards(&mut self.veil, open, VEIL_IN, VEIL_OUT);
        towards(&mut self.menu, open, MENU_IN, MENU_OUT);
        towards(&mut self.options, open && options, VEIL_IN, VEIL_OUT);
    }

    /// Another screen takes the buttons' place: it comes as they did, over a world that
    /// stays as it is.
    pub fn change_screen(&mut self) {
        self.menu = 0.0;
    }

    /// Closed at once (the world is gone).
    pub fn end(&mut self) {
        *self = MenuTransition::default();
    }

    /// Whether anything of it shows.
    pub fn shows(&self) -> bool {
        self.veil > 0.0 || self.menu > 0.0
    }

    /// Whether the buttons show.
    pub fn menu_shows(&self) -> bool {
        self.menu > 0.0
    }

    /// How to draw it, with `px` pixels to a UI unit. `still` (reduced motion): the
    /// buttons fade without the blur.
    pub fn look(&self, px: f32, still: bool) -> MenuLook {
        let v = smooth(self.veil);
        let m = smooth(self.menu);
        let dim = BRIGHTNESS + (BRIGHTNESS_OPTIONS - BRIGHTNESS) * smooth(self.options);
        MenuLook {
            saturation: 1.0 + (SATURATION - 1.0) * v,
            brightness: 1.0 + (dim - 1.0) * v,
            blur: BLUR * px * v,
            menu_blur: if still {
                0.0
            } else {
                MENU_BLUR * px * (1.0 - m)
            },
            menu_opacity: smooth(self.menu / OPAQUE_AT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut MenuTransition, open: bool, seconds: f32) {
        for _ in 0..(seconds / 0.001).round() as usize {
            t.step(open, false, 0.001);
        }
    }

    #[test]
    fn closed_it_changes_nothing_and_open_the_menu_is_sharp() {
        let mut t = MenuTransition::default();
        assert!(!t.shows() && !t.menu_shows());
        let closed = t.look(1.0, false);
        assert_eq!(
            (closed.saturation, closed.brightness, closed.blur),
            (1.0, 1.0, 0.0)
        );
        assert_eq!(closed.menu_opacity, 0.0);
        run(&mut t, true, 0.25);
        let open = t.look(2.0, false);
        assert_eq!((open.menu_blur, open.menu_opacity), (0.0, 1.0));
        assert_eq!(open.blur, 2.0 * BLUR);
        assert!((open.saturation - SATURATION).abs() < 1e-6);
        assert!((open.brightness - BRIGHTNESS).abs() < 1e-6);
        run(&mut t, false, 0.2);
        assert!(!t.shows());
        assert_eq!(t.look(1.0, false), closed);
    }

    #[test]
    fn it_is_over_in_a_fifth_of_a_second_and_goes_faster_than_it_comes() {
        let mut t = MenuTransition::default();
        run(&mut t, true, 0.2);
        assert_eq!(t, MenuTransition::at(1.0));
        run(&mut t, false, 0.12);
        assert!(!t.menu_shows() && t.shows(), "the buttons go first");
        run(&mut t, false, 0.04);
        assert!(!t.shows());
    }

    #[test]
    fn it_ramps_smoothly_and_the_same_at_any_frame_rate() {
        let mut t = MenuTransition::default();
        let mut last = t.look(1.0, false);
        for _ in 0..200 {
            t.step(true, false, 0.001);
            let l = t.look(1.0, false);
            // Never by much from one millisecond to the next, and never back.
            assert!((0.0..0.01).contains(&(last.saturation - l.saturation)));
            assert!((0.0..0.16).contains(&(last.menu_blur - l.menu_blur)));
            assert!(l.menu_opacity >= last.menu_opacity);
            last = l;
        }
        // Slow at both ends: half of the way in the middle third or so of the time.
        let sat = |share: f32| MenuTransition::at(share).look(1.0, false).saturation;
        let gone = |share: f32| (1.0 - sat(share)) / (1.0 - SATURATION);
        assert!(gone(0.1) < 0.05 && gone(0.9) > 0.95);
        assert!((gone(0.5) - 0.5).abs() < 1e-4);
        let (mut a, mut b) = (MenuTransition::default(), MenuTransition::default());
        a.step(true, false, 0.09);
        for _ in 0..9 {
            b.step(true, false, 0.01);
        }
        assert!((a.look(1.0, false).blur - b.look(1.0, false).blur).abs() < 1e-4);
    }

    #[test]
    fn turned_round_it_goes_on_from_where_it_is() {
        let mut t = MenuTransition::default();
        run(&mut t, true, 0.08);
        let before = t.look(1.0, false);
        t.step(false, false, 0.0);
        assert_eq!(t.look(1.0, false), before);
        t.step(false, false, 0.001);
        let after = t.look(1.0, false);
        assert!(after.saturation > before.saturation);
        assert!((after.saturation - before.saturation).abs() < 0.02);
    }

    #[test]
    fn another_screen_comes_over_the_same_world() {
        let mut t = MenuTransition::default();
        run(&mut t, true, 0.25);
        let world = t.look(1.0, false);
        t.change_screen();
        let l = t.look(1.0, false);
        assert_eq!((l.saturation, l.blur), (world.saturation, world.blur));
        assert_eq!((l.menu_opacity, l.menu_blur), (0.0, MENU_BLUR));
        assert!(t.shows());
        // The options dim the world further, and the menu brightens it again.
        let mut last = world.brightness;
        for _ in 0..180 {
            t.step(true, true, 0.001);
            let b = t.look(1.0, false).brightness;
            assert!(b <= last && last - b < 0.01);
            last = b;
        }
        assert!((last - BRIGHTNESS_OPTIONS).abs() < 1e-4);
        assert_eq!(t.look(1.0, false).saturation, world.saturation);
        run(&mut t, true, 0.2);
        assert!((t.look(1.0, false).brightness - BRIGHTNESS).abs() < 1e-4);
        t.end();
        assert!(!t.shows());
        // Reduced motion fades without the blur.
        run(&mut t, true, 0.05);
        let l = t.look(1.0, true);
        assert!(l.menu_blur == 0.0 && l.menu_opacity > 0.0 && l.menu_opacity < 1.0);
    }
}
