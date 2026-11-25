//! The player's own hands as its eyes see them, and how its view bobs and tilts
//! (`FirstPersonHandsAndItemsRenderer`, `GameRenderer.bobHurt` and `bobView`,
//! `SpearAnimations.firstPersonAttack`).
//!
//! Everything here is in the view's own space: x to the right, y up, and what is ahead at
//! negative z. A transform is built as the game builds its pose stack: what is said last
//! is done first.

use ruthenium_javamath::mth;
use ruthenium_sim::first_person::FirstPersonRecord;
use ruthenium_sim::item::{Class, SpearTiming, animation};

use super::affine::Affine;

const PI: f32 = std::f32::consts::PI;

fn sin(v: f32) -> f32 {
    mth::sin(f64::from(v))
}

fn cos(v: f32) -> f32 {
    mth::cos(f64::from(v))
}

fn sqrt(v: f32) -> f32 {
    f64::from(v).sqrt() as f32
}

/// Degrees as the game turns its pose stack by them.
fn deg(degrees: f32) -> f32 {
    degrees * 0.017_453_292
}

/// `GameRenderer.bobHurt`: a dying player's view falls to its side, and one that is hurt
/// tilts away from where the hurt came from. `strength` is the Damage Tilt option.
pub(crate) fn bob_hurt(m: Affine, r: &FirstPersonRecord, partial: f32, strength: f32) -> Affine {
    let mut m = m;
    if r.dead {
        let duration = (r.death_time as f32 + partial).min(20.0);
        m = m.rotate_z(deg(40.0 - 8000.0 / (duration + 200.0)));
    }
    let hurt = r.hurt_time as f32 - partial;
    if hurt < 0.0 || r.hurt_duration == 0 {
        return m;
    }
    let hurt = hurt / r.hurt_duration as f32;
    let hurt = sin(hurt * hurt * hurt * hurt * PI);
    let tilt = (f64::from(-hurt) * 14.0 * f64::from(strength)) as f32;
    m.rotate_y(deg(-r.hurt_dir))
        .rotate_z(deg(tilt))
        .rotate_y(deg(r.hurt_dir))
}

/// `GameRenderer.bobView`: the view sways and nods with the player's steps.
pub(crate) fn bob_view(m: Affine, r: &FirstPersonRecord, partial: f32) -> Affine {
    let (old, now) = r.walk;
    // `getBackwardsInterpolatedWalkDistance`.
    let walked = -(now + (now - old) * partial);
    let bob = mth::lerp(partial, r.bob.0, r.bob.1);
    m.translate([
        sin(walked * PI) * bob * 0.5,
        -(cos(walked * PI) * bob).abs(),
        0.0,
    ])
    .rotate_z(deg(sin(walked * PI) * bob * 3.0))
    .rotate_x(deg((cos(walked * PI - 0.2) * bob).abs() * 5.0))
}

/// What a hand shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Shown {
    Nothing,
    /// The bare arm: where its part of the player's model is drawn from.
    Arm(Affine),
    /// What the hand holds: where its model is drawn from, before the model's own place
    /// in a hand.
    Item(Affine),
}

/// The player's hands at one moment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hands<'a> {
    pub r: &'a FirstPersonRecord,
    pub partial: f32,
    /// The view's pitch and yaw, in degrees.
    pub x_rot: f32,
    pub y_rot: f32,
}

impl Hands<'_> {
    /// `submitHandsWithItems`' start: the hands trail behind the view as it turns.
    pub fn trailing(&self, m: Affine) -> Affine {
        let r = self.r;
        let x_bob = mth::lerp(self.partial, r.x_bob.0, r.x_bob.1);
        let y_bob = mth::lerp(self.partial, r.y_bob.0, r.y_bob.1);
        m.rotate_x(deg((self.x_rot - x_bob) * 0.1))
            .rotate_y(deg((self.y_rot - y_bob) * 0.1))
    }

    /// How far the swing under way has come, 0 to 1 (`SwingState.getAnimation`).
    pub fn swing(&self) -> f32 {
        let (old, now) = self.r.swing_progress;
        let to = if now >= old { now } else { now + 1.0 };
        mth::lerp(self.partial, old, to)
    }

    /// Whether the hand is the right one.
    pub fn is_right(&self, off_hand: bool) -> bool {
        off_hand == self.r.left_handed
    }

    /// `submitArmWithItem`: what the main hand or the other one shows, and where, from
    /// `m`. `swap_scale` is how much farther than usual the hand's item sinks out of
    /// sight (`swapAnimationScale`).
    pub fn hand(&self, m: Affine, off_hand: bool, swap_scale: f32) -> Shown {
        let r = self.r;
        if r.scoping {
            return Shown::Nothing;
        }
        let hand = usize::from(off_hand);
        let stack = &r.held[hand];
        let (old, now) = r.heights[hand];
        let lowered = swap_scale * (1.0 - mth::lerp(self.partial, old, now));
        // The swing is of the hand that swings; with none under way, of the main one.
        let swinging = r.swing.is_some_and(|s| s.off_hand) == off_hand;
        let attack = if swinging { self.swing() } else { 0.0 };
        let right = self.is_right(off_hand);
        let side = if right { 1.0 } else { -1.0 };
        if stack.is_empty() {
            return if !off_hand && !r.invisible {
                Shown::Arm(arm(m, lowered, attack, side))
            } else {
                Shown::Nothing
            };
        }
        let c = &stack.c;
        let using = r.using == Some(off_hand) && r.use_remaining > 0;
        // How long it has been used for.
        let held = r.use_duration[hand] as f32 - (r.use_remaining as f32 - self.partial + 1.0);
        if c.class == Class::Crossbow {
            let m = at_rest(m, side, lowered);
            return Shown::Item(if using && !c.charged {
                let m = m
                    .translate([side * -0.478_568_2, -0.094_387, 0.057_315_31])
                    .rotate_x(deg(-11.935))
                    .rotate_y(deg(side * 65.3))
                    .rotate_z(deg(side * -9.785));
                let power = (held / r.charge_duration[hand] as f32).min(1.0);
                drawn(m, held, power, side, 0.04)
            } else {
                let m = swung(m, attack, side);
                if c.charged && attack < 0.001 && !off_hand {
                    m.translate([side * -0.641_864, 0.0, 0.0])
                        .rotate_y(deg(side * 10.0))
                } else {
                    m
                }
            });
        }
        Shown::Item(if using {
            let kind = c.use_animation();
            // Eating, drinking and a spear place the hand themselves.
            let own = matches!(kind, animation::EAT | animation::DRINK | animation::SPEAR);
            let m = if own { m } else { at_rest(m, side, lowered) };
            match kind {
                animation::EAT | animation::DRINK => {
                    let m = eating(m, self.partial, side, r.use_remaining, r.use_duration[hand]);
                    at_rest(m, side, lowered)
                }
                animation::BLOCK if !c.is("ShieldItem") => m
                    .translate([side * -0.141_421_36, 0.08, 0.141_421_36])
                    .rotate_x(deg(-102.25))
                    .rotate_y(deg(side * 13.365))
                    .rotate_z(deg(side * 78.05)),
                animation::BOW => {
                    let m = m
                        .translate([side * -0.278_568_2, 0.183_443_87, 0.157_315_31])
                        .rotate_x(deg(-13.935))
                        .rotate_y(deg(side * 35.3))
                        .rotate_z(deg(side * -9.785));
                    let power = held / 20.0;
                    let power = ((power * power + power * 2.0) / 3.0).min(1.0);
                    drawn(m, held, power, side, 0.04)
                }
                animation::TRIDENT => {
                    let m = m
                        .translate([side * -0.5, 0.7, 0.1])
                        .rotate_x(deg(-55.0))
                        .rotate_y(deg(side * 35.3))
                        .rotate_z(deg(side * -9.785));
                    drawn(m, held, (held / 10.0).min(1.0), side, 0.2)
                }
                animation::BRUSH => brushing(m, self.partial, right, r.use_remaining),
                animation::BUNDLE => swung(m, attack, side),
                animation::SPEAR => {
                    let m = at_rest(m, side, 0.0);
                    c.spear.map_or(m, |timing| {
                        spear_use(
                            m,
                            timing,
                            held,
                            side,
                            r.ticks_since_kinetic_hit
                                .map_or(0.0, |t| t as f32 + self.partial),
                        )
                    })
                }
                _ => m,
            }
        } else if r.spin_attack {
            at_rest(m, side, lowered)
                .translate([side * -0.4, 0.8, 0.3])
                .rotate_y(deg(side * 65.0))
                .rotate_z(deg(side * -85.0))
        } else {
            let m = at_rest(m, side, lowered);
            match r.swing.filter(|s| s.off_hand == off_hand).map(|s| s.kind) {
                Some(1) => swung(m, attack, side),
                Some(2) => stabbing(m, attack, side),
                _ => m,
            }
        })
    }
}

/// `applyItemArmTransform`: where a hand's item rests, `lowered` out of sight.
fn at_rest(m: Affine, side: f32, lowered: f32) -> Affine {
    m.translate([side * 0.56, -0.52 + lowered * -0.6, -0.72])
}

/// `swingArm`: a whack.
fn swung(m: Affine, attack: f32, side: f32) -> Affine {
    let root = sqrt(attack);
    let m = m.translate([
        side * -0.4 * sin(root * PI),
        0.2 * sin(root * (PI * 2.0)),
        -0.2 * sin(attack * PI),
    ]);
    // `applyItemArmAttackTransform`.
    let turn = sin(attack * attack * PI);
    let swing = sin(root * PI);
    m.rotate_y(deg(side * (45.0 + turn * -20.0)))
        .rotate_z(deg(side * swing * -20.0))
        .rotate_x(deg(swing * -80.0))
        .rotate_y(deg(side * -45.0))
}

/// `Ease.inOutSine`, `outBack` and `inOutExpo`.
fn in_out_sine(x: f32) -> f32 {
    -(cos(PI * x) - 1.0) / 2.0
}

fn out_back(x: f32) -> f32 {
    let x = x - 1.0;
    1.0 + 2.701_58 * (x * x * x) + 1.701_58 * (x * x)
}

fn in_out_expo(x: f32) -> f32 {
    let x64 = f64::from(x);
    if x < 0.5 {
        if x == 0.0 {
            0.0
        } else {
            (2.0f64.powf(20.0 * x64 - 10.0) / 2.0) as f32
        }
    } else if x == 1.0 {
        1.0
    } else {
        ((2.0 - 2.0f64.powf(-20.0 * x64 + 10.0)) / 2.0) as f32
    }
}

/// How far `time` is from `start` to `end`, 0 to 1 (`SpearAnimations.progress`).
fn progress(time: f32, start: f32, end: f32) -> f32 {
    ((time - start) / (end - start)).clamp(0.0, 1.0)
}

/// `SpearAnimations.firstPersonAttack`: a stab.
fn stabbing(m: Affine, attack: f32, side: f32) -> Affine {
    let starting = in_out_sine(progress(attack, 0.0, 0.05));
    let middle = out_back(progress(attack, 0.05, 0.2));
    let ending = in_out_expo(progress(attack, 0.4, 1.0));
    m.translate([
        side * 0.1 * (starting - middle),
        -0.075 * (starting - ending),
        0.65 * (starting - middle),
    ])
    .rotate_x(deg(-70.0 * (starting - ending)))
    .translate([0.0, 0.0, (-0.25 * f64::from(ending - middle)) as f32])
}

/// The ready spear pitches about a point above the grip and turns about a point
/// beside it. Its component sets the end of each movement relative to the raise.
fn spear_use(m: Affine, timing: SpearTiming, held: f32, side: f32, feedback: f32) -> Affine {
    let delay = timing.delay as f32;
    let phase = |duration: i32, length: f32, t: f32| {
        let end = delay + duration as f32;
        progress(t, end - length, end)
    };
    let raised = progress(held, 0.0, delay);
    let middle = progress(raised, 0.5, 0.8);
    let end = progress(raised, 0.8, 1.0);
    let sway = phase(timing.dismount, 20.0, held);
    let back = phase(timing.damage, 5.0, held);
    let amplitude = 2.0 * (sqrt(1.0 - (sway - 1.0).powi(2)) + sqrt(1.0 - back * back) - 1.0);
    let slow = sin(deg(held * 19.0)) * amplitude;
    let fast = sin(deg(held * 30.0)) * amplitude;

    // In-out back for the raise, and out-cubic of in-out elastic for lowering.
    let t = if raised < 0.5 {
        2.0 * raised
    } else {
        2.0 * (1.0 - raised)
    };
    let overshoot = 0.5 * t * t * (3.5949094 * t - 2.5949094);
    let tilt = if raised < 0.5 {
        overshoot
    } else {
        1.0 - overshoot
    };
    let lower = phase(timing.knockback, 40.0, held - 20.0);
    let elastic = if lower > 0.0 && lower < 1.0 {
        let t = 20.0 * f64::from(lower);
        let wave = ((t - 11.125) * f64::from(1.3962635_f32)).sin() * 0.5;
        if lower < 0.5 {
            (-wave * (t - 10.0).exp2()) as f32
        } else {
            (1.0 + wave * (10.0 - t).exp2()) as f32
        }
    } else {
        lower
    };
    let lowered = 1.0 - (1.0 - elastic).powi(3);
    let recoil = 0.4
        * (1.0
            - (1.0 - progress(feedback, 1.0, 3.0)).powi(4)
            - in_out_sine(progress(feedback, 3.0, 10.0)));
    let yaw_pivot = side * 0.15;
    m.translate([
        side * (0.15 * raised - 0.05 * end - 0.1 * sway + 0.005 * slow),
        0.075 * (middle - raised) + 0.01 * fast,
        0.05 * (progress(raised, 0.0, 0.5) - end) + 0.005 * slow,
    ])
    .translate([0.0, 0.1, 0.0])
    .rotate_x(deg(
        -65.0 * tilt - 35.0 * lowered + 100.0 * back - 0.5 * fast
    ))
    .translate([yaw_pivot, -0.1, 0.0])
    .rotate_y(deg(
        side * (90.0 * (progress(raised, 0.5, 0.55) - sway) - 2.0 * slow)
    ))
    .translate([-yaw_pivot, -recoil, 0.0])
}

/// The end of drawing a bow, loading a crossbow or raising a trident: it shakes once it
/// is drawn a tenth of the way, is pulled back by `pull` and stretched.
fn drawn(m: Affine, held: f32, power: f32, side: f32, pull: f32) -> Affine {
    let mut m = m;
    if power > 0.1 {
        let shake = sin((held - 0.1) * 1.3) * (power - 0.1);
        m = m.translate([0.0, shake * 0.004, 0.0]);
    }
    m.translate([0.0, 0.0, power * pull])
        .scale([1.0, 1.0, 1.0 + power * 0.2])
        .rotate_y(deg(-(side * 45.0)))
}

/// `applyEatTransform`.
fn eating(m: Affine, partial: f32, side: f32, remaining: i32, duration: i32) -> Affine {
    let left = remaining as f32 - partial + 1.0;
    let share = left / duration as f32;
    let mut m = m;
    if share < 0.8 {
        m = m.translate([0.0, (cos(left / 4.0 * PI) * 0.1).abs(), 0.0]);
    }
    let jiggle = 1.0 - f64::from(share).powf(27.0) as f32;
    m.translate([jiggle * 0.6 * side, jiggle * -0.5, 0.0])
        .rotate_y(deg(side * jiggle * 90.0))
        .rotate_x(deg(jiggle * 10.0))
        .rotate_z(deg(side * jiggle * 30.0))
}

/// `applyBrushTransform`.
fn brushing(m: Affine, partial: f32, right: bool, remaining: i32) -> Affine {
    let left = (remaining % 10) as f32 - partial + 1.0;
    let share = 1.0 - left / 10.0;
    let swipe = -15.0 + 75.0 * cos(share * 2.0 * PI);
    if right {
        m.translate([-0.25, 0.22, 0.35])
            .rotate_x(deg(-80.0))
            .rotate_y(deg(90.0))
            .rotate_x(deg(swipe))
    } else {
        m.translate([0.1, 0.83, 0.35])
            .rotate_x(deg(-80.0))
            .rotate_y(deg(-90.0))
            .rotate_x(deg(swipe))
            .translate([-0.3, 0.22, 0.35])
    }
}

/// `renderPlayerArm`: the bare arm, swinging with `attack`.
fn arm(m: Affine, lowered: f32, attack: f32, side: f32) -> Affine {
    let root = sqrt(attack);
    m.translate([
        side * (-0.3 * sin(root * PI) + 0.640_000_05),
        0.4 * sin(root * (PI * 2.0)) + -0.6 + lowered * -0.6,
        -0.4 * sin(attack * PI) + -0.719_999_97,
    ])
    .rotate_y(deg(side * 45.0))
    .rotate_y(deg(side * sin(root * PI) * 70.0))
    .rotate_z(deg(side * sin(attack * attack * PI) * -20.0))
    .translate([-side, 3.6, 3.5])
    .rotate_z(deg(side * 120.0))
    .rotate_x(deg(200.0))
    .rotate_y(deg(side * -135.0))
    .translate([side * 5.6, 0.0, 0.0])
}

/// The stack as [`Hands`] needs it for a test.
#[cfg(test)]
pub(crate) fn test_stack(
    class: Class,
    set: impl FnOnce(&mut ruthenium_sim::item::Components),
) -> ruthenium_sim::item::Stack {
    use ruthenium_sim::item::Stack;
    let mut c = ruthenium_sim::item::Components {
        class,
        ..Default::default()
    };
    set(&mut c);
    Stack {
        identity: 1,
        item: 1,
        count: 1,
        c: std::sync::Arc::new(c),
        ..Stack::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_sim::avatar::Swing;
    use ruthenium_sim::item::Stack;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    fn record() -> FirstPersonRecord {
        FirstPersonRecord {
            heights: [(1.0, 1.0); 2],
            ..FirstPersonRecord::default()
        }
    }

    fn hands(r: &FirstPersonRecord) -> Hands<'_> {
        Hands {
            r,
            partial: 0.5,
            x_rot: 0.0,
            y_rot: 0.0,
        }
    }

    fn item(m: Shown) -> Affine {
        match m {
            Shown::Item(m) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_item_rests_low_to_the_side_of_its_hand_and_sinks_out_of_sight() {
        let mut r = record();
        r.held = [
            test_stack(Class::Plain, |_| {}),
            test_stack(Class::Plain, |_| {}),
        ];
        let h = hands(&r);
        let main = item(h.hand(Affine::IDENTITY, false, 1.0));
        assert!(close(main.point([0.0; 3]), [0.56, -0.52, -0.72]));
        // The other hand on the other side; a left-handed player's the other way round.
        let off = item(h.hand(Affine::IDENTITY, true, 1.0));
        assert!(close(off.point([0.0; 3]), [-0.56, -0.52, -0.72]));
        let mut left = r.clone();
        left.left_handed = true;
        let main = item(hands(&left).hand(Affine::IDENTITY, false, 1.0));
        assert!(close(main.point([0.0; 3]), [-0.56, -0.52, -0.72]));
        // Half way down between two ticks, and farther for an item that sinks farther.
        r.heights[0] = (1.0, 0.0);
        let h = hands(&r);
        let sunk = item(h.hand(Affine::IDENTITY, false, 1.0));
        assert!(close(sunk.point([0.0; 3]), [0.56, -0.82, -0.72]));
        let sunk = item(h.hand(Affine::IDENTITY, false, 1.95));
        assert!(close(sunk.point([0.0; 3]), [0.56, -0.52 - 0.585, -0.72]));
    }

    #[test]
    fn only_the_hand_that_swings_swings() {
        let mut r = record();
        r.held = [
            test_stack(Class::Plain, |_| {}),
            test_stack(Class::Plain, |_| {}),
        ];
        r.swing = Some(Swing {
            off_hand: false,
            kind: 1,
            duration: 6,
        });
        r.swing_progress = (0.25, 0.5);
        let h = hands(&r);
        assert!((h.swing() - 0.375).abs() < 1e-6);
        let rest = at_rest(Affine::IDENTITY, 1.0, 0.0);
        assert_ne!(item(h.hand(Affine::IDENTITY, false, 1.0)), rest);
        assert_eq!(
            item(h.hand(Affine::IDENTITY, true, 1.0)),
            at_rest(Affine::IDENTITY, -1.0, 0.0)
        );
        // A swing that ended this tick goes on to its end, not back to its start.
        r.swing_progress = (0.9, 0.0);
        assert!((hands(&r).swing() - 0.95).abs() < 1e-6);
        // A swing that is none moves nothing; a stab is not a whack.
        r.swing_progress = (0.25, 0.5);
        r.swing.as_mut().unwrap().kind = 0;
        assert_eq!(item(hands(&r).hand(Affine::IDENTITY, false, 1.0)), rest);
        r.swing.as_mut().unwrap().kind = 2;
        let stab = item(hands(&r).hand(Affine::IDENTITY, false, 1.0));
        r.swing.as_mut().unwrap().kind = 1;
        let whack = item(hands(&r).hand(Affine::IDENTITY, false, 1.0));
        assert!(stab != whack && stab != rest);
    }

    #[test]
    fn hand_transforms_match_the_game() {
        // Captured from the 26.3 renderer with HandGolden. Each matrix consists of
        // three affine rows in view space, before applying the item's display transform.
        #[rustfmt::skip]
        let expected: [[f32; 12]; 10] = [
            [-0.633458, -0.62889504, 0.4508014, 0.24575448, -0.74657106, 0.34361857, -0.56969976, -1.4516485, 0.2033776, -0.6974362, -0.6871828, -0.7625046],
            [0.82361126, -0.5073552, -0.2534859, -0.37533492, 0.41941553, 0.24398993, 0.8743909, -0.12976126, -0.38177878, -0.8264742, 0.41374552, -0.1847759],
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.34202018, 0.9396926, 0.15992315, 0.0, -0.9396926, 0.34202018, 0.085505046],
            [0.08682409, 0.15038373, 0.9848077, 0.6, 0.49240386, 0.8528685, -0.17364818, -0.46173167, -0.8660254, 0.5, -0.0, 0.0],
            [0.0, -0.8579414, 0.5137476, -0.25, -0.9848077, 0.08921142, 0.14898011, 0.22, -0.17364836, -0.5059426, -0.8449073, 0.35],
            [-0.633458, 0.62889504, -0.4508014, -0.24575448, 0.74657106, 0.34361857, -0.56969976, -1.4516485, -0.2033776, -0.6974362, -0.6871828, -0.7625046],
            [0.82361126, 0.5073552, 0.2534859, 0.37533492, -0.41941553, 0.24398993, 0.8743909, -0.12976126, 0.38177878, -0.8264742, 0.41374552, -0.1847759],
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.34202018, 0.9396926, 0.15992315, 0.0, -0.9396926, 0.34202018, 0.085505046],
            [0.08682409, -0.15038373, -0.9848077, -0.6, -0.49240386, 0.8528685, -0.17364818, -0.46173167, 0.8660254, 0.5, 0.0, 0.0],
            [0.0, 0.8579414, -0.5137476, 0.10893547, 0.9848077, 0.08921142, 0.14898011, 0.6063272, 0.17364836, -0.5059426, -0.8449073, -0.109119415],
        ];
        let m = Affine::IDENTITY;
        let poses = [1.0, -1.0].into_iter().flat_map(|side| {
            [
                arm(m, 0.2, 0.375, side),
                swung(m, 0.375, side),
                stabbing(m, 0.375, side),
                eating(m, 0.5, side, 13, 32),
                brushing(m, 0.5, side > 0.0, 13),
            ]
        });
        for (index, (pose, reference)) in poses.zip(expected).enumerate() {
            for (actual, want) in pose.0.iter().flatten().zip(reference) {
                assert!(
                    (actual - want).abs() < 0.00001,
                    "pose {index}: {actual} vs {want}"
                );
            }
        }
    }

    #[test]
    fn ready_spear_matches_the_games_raise_sway_lower_and_recovery() {
        let timing = SpearTiming {
            delay: 15,
            dismount: 100,
            knockback: 200,
            damage: 300,
        };
        // HandGolden, left arm, with a hit two and a half ticks ago.
        #[rustfmt::skip]
        let cases = [
            (3.5, [0.99999994, 0.0, 0.0, -0.034999996, 0.0, 0.99358696, -0.11307036, -0.412741, 0.0, 0.11307036, 0.99358696, -0.033025175]),
            (15.5, [0.0, 0.0, -0.99999994, -0.25, 0.90630776, 0.42261815, 0.0, 0.025297433, 0.42261815, -0.90630776, 0.0, 0.5151305]),
            (109.5, [0.87855554, 0.0, -0.47764054, -0.036263168, 0.43525374, 0.41183618, 0.8005907, -0.026390955, 0.19670963, -0.91125804, 0.3618209, 0.4742581]),
            (201.5, [0.9986334, 0.0, -0.05226246, 0.0072844625, 0.046789378, 0.44550997, 0.8940535, -0.13435932, 0.023283446, -0.895277, 0.44490114, 0.44224268]),
            (312.5, [0.9999965, 0.0, 0.002642372, -3.7902594E-4, -0.0020308017, 0.63978654, 0.7685499, -0.21471642, -0.0016905542, -0.76855254, 0.63978434, 0.38320032]),
        ];
        for (held, reference) in cases {
            let m = spear_use(Affine::IDENTITY, timing, held, -1.0, 2.5);
            for (actual, want) in m.0.iter().flatten().zip(reference) {
                assert!(
                    (actual - want).abs() < 1e-5,
                    "tick {held}: {actual} vs {want}"
                );
            }
        }
    }

    #[test]
    fn a_whack_begins_and_ends_at_rest() {
        let rest = Affine::IDENTITY;
        for attack in [0.0, 1.0] {
            let m = swung(rest, attack, 1.0);
            for (row, want) in m.0.iter().zip(rest.0) {
                for (a, b) in row.iter().zip(want) {
                    assert!((a - b).abs() < 1e-3, "{attack}: {m:?}");
                }
            }
        }
        // Half way it is turned down and drawn in.
        let mid = swung(rest, 0.5, 1.0);
        assert!(mid.point([0.0; 3])[0] < -0.3);
    }

    #[test]
    fn the_bare_arm_is_the_main_hands_and_hidden_when_unseen() {
        let mut r = record();
        let h = hands(&r);
        let Shown::Arm(m) = h.hand(Affine::IDENTITY, false, 1.0) else {
            panic!("no arm");
        };
        // Its end reaches into view from below on the right.
        let hand = m.point([-6.0 / 16.0, 12.0 / 16.0, 0.0]);
        assert!(hand[0] > 0.0 && hand[1] < 0.0 && hand[2] < 0.0, "{hand:?}");
        assert_eq!(h.hand(Affine::IDENTITY, true, 1.0), Shown::Nothing);
        r.invisible = true;
        assert_eq!(hands(&r).hand(Affine::IDENTITY, false, 1.0), Shown::Nothing);
        // Through a spyglass nothing is held up.
        r.invisible = false;
        r.scoping = true;
        assert_eq!(hands(&r).hand(Affine::IDENTITY, false, 1.0), Shown::Nothing);
    }

    #[test]
    fn using_an_item_moves_it_as_its_use_says() {
        let mut r = record();
        let bow = test_stack(Class::Bow, |_| {});
        r.held = [bow, Stack::empty()];
        r.use_duration = [72000, 0];
        let rest = at_rest(Affine::IDENTITY, 1.0, 0.0);
        assert_eq!(item(hands(&r).hand(Affine::IDENTITY, false, 1.0)), rest);
        // Drawn for 10.5 ticks.
        r.using = Some(false);
        r.use_remaining = 72000 - 10;
        let drawn = item(hands(&r).hand(Affine::IDENTITY, false, 1.0));
        assert_ne!(drawn, rest);
        // Stretched along its length by a fifth of how far it is drawn.
        let held = 10.5f32;
        let power = held / 20.0;
        let power = (power * power + power * 2.0) / 3.0;
        let length = |m: &Affine| {
            let (a, b) = (m.point([0.0; 3]), m.point([1.0, 0.0, 1.0]));
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        assert!(length(&drawn) > 2.0f32.sqrt() && power > 0.1);
        // The other hand's use is not this one's.
        r.using = Some(true);
        assert_eq!(item(hands(&r).hand(Affine::IDENTITY, false, 1.0)), rest);
        // A shield stays where it rests; another blocking item is turned across.
        let mut r = record();
        r.using = Some(false);
        r.use_remaining = 100;
        r.held[0] = test_stack(Class::Plain, |c| {
            c.blocks_attacks = true;
            c.classes = std::sync::Arc::from(vec!["ShieldItem".to_owned()]);
        });
        assert_eq!(item(hands(&r).hand(Affine::IDENTITY, false, 1.0)), rest);
        r.held[0] = test_stack(Class::Plain, |c| c.blocks_attacks = true);
        assert_ne!(item(hands(&r).hand(Affine::IDENTITY, false, 1.0)), rest);
    }

    #[test]
    fn a_loaded_crossbow_is_held_across_by_the_main_hand() {
        let mut r = record();
        r.held[0] = test_stack(Class::Crossbow, |c| c.charged = true);
        r.charge_duration = [25, 25];
        let m = item(hands(&r).hand(Affine::IDENTITY, false, 1.0));
        assert!(close(m.point([0.0; 3]), [0.56 - 0.641_864, -0.52, -0.72]));
        // Not while it swings, nor in the other hand.
        r.held[1] = r.held[0].clone();
        let off = item(hands(&r).hand(Affine::IDENTITY, true, 1.0));
        assert!(close(off.point([0.0; 3]), [-0.56, -0.52, -0.72]));
    }

    #[test]
    fn the_view_tilts_when_hurt_and_bobs_with_steps() {
        let mut r = record();
        assert_eq!(bob_hurt(Affine::IDENTITY, &r, 0.5, 1.0), Affine::IDENTITY);
        // Hurt from the left (yaw 90): at the first moment the tilt is none yet (the
        // sine of a whole turn), half way it leans.
        r.hurt_time = 10;
        r.hurt_duration = 10;
        r.hurt_dir = 90.0;
        let start = bob_hurt(Affine::IDENTITY, &r, 0.0, 1.0);
        assert!(close(start.point([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]));
        r.hurt_time = 7;
        let tilted = bob_hurt(Affine::IDENTITY, &r, 0.0, 1.0);
        let up = tilted.point([0.0, 1.0, 0.0]);
        assert!(up[2].abs() > 0.1 && up[0].abs() < 1e-4, "{up:?}");
        // No tilt with the option at nothing.
        assert!(close(
            bob_hurt(Affine::IDENTITY, &r, 0.0, 0.0).point([0.0, 1.0, 0.0]),
            [0.0, 1.0, 0.0]
        ));
        // Dying, the view falls over by up to 3.6 degrees in a second.
        let mut dead = record();
        dead.dead = true;
        dead.death_time = 20;
        dead.hurt_time = 0;
        let fallen = bob_hurt(Affine::IDENTITY, &dead, 1.0, 1.0);
        let angle = 40.0f32 - 8000.0 / 220.0;
        assert!((fallen.point([1.0, 0.0, 0.0])[1] - angle.to_radians().sin()).abs() < 1e-4);
        // Standing still nothing bobs; walking, the view sinks and sways.
        assert_eq!(bob_view(Affine::IDENTITY, &r, 0.5), Affine::IDENTITY);
        r.bob = (0.1, 0.1);
        r.walk = (0.25, 0.25);
        let m = bob_view(Affine::IDENTITY, &r, 0.5);
        let at = m.point([0.0; 3]);
        assert!(at[0] < -0.03 && at[1] < -0.06, "{at:?}");
    }

    #[test]
    fn the_hands_trail_the_view_by_a_tenth() {
        let mut r = record();
        r.x_bob = (0.0, 10.0);
        r.y_bob = (0.0, 0.0);
        let h = Hands {
            r: &r,
            partial: 1.0,
            x_rot: 30.0,
            y_rot: 0.0,
        };
        // Two degrees behind the pitch.
        let m = h.trailing(Affine::IDENTITY);
        let ahead = m.point([0.0, 0.0, -1.0]);
        assert!(
            (ahead[1] - 2.0f32.to_radians().sin()).abs() < 1e-4,
            "{ahead:?}"
        );
    }
}
