//! Where a sound is heard from, as OpenAL Soft (the 1.25 that LWJGL 3.4.3 bundles, which
//! vanilla 26.3 runs on) computes it for plain stereo output.
//!
//! Distance: vanilla gives every positioned sound `AL_LINEAR_DISTANCE_CLAMPED` with a
//! reference distance of 0 and a rolloff of 1, so the gain is `1 - d / max`, clamped to
//! 0..1 (`Channel.linearAttenuation`); others have no distance model.
//!
//! Direction: OpenAL Soft pans through first-order ambisonics. A mono source's direction
//! in the listener's space (x right, y up, z behind) is widened in front
//! (`ScaleAzimuthFront3_2`: +-60 degrees become +-90, anything wider in front +-90),
//! encoded with N3D coefficients (W = 1, Y = sqrt3 * left, X = sqrt3 * front) and
//! decoded by its `StereoConfig` (`{0.5, +-0.288675135, 0.0552305643}` on W, Y, X):
//!
//! `left = 0.5 + 0.5 * l + 0.0956621 * f`, `right = 0.5 - 0.5 * l + 0.0956621 * f`
//!
//! so a sound hard left is (1, 0), one ahead (0.596, 0.596), one behind (0.404, 0.404),
//! and a sound at the listener (UI clicks, music) is treated as ahead. This is
//! OpenAL Soft's own law rather than a textbook one, and it is used because it is what
//! vanilla's players hear: a constant-power pan would be 3 dB louder at the sides.
//! Stereo sources are not placed or attenuated at all: their channels go to the left
//! and right speakers at the source's gain (`CalcNonAttnVoiceParams` with the stereo
//! map at +-30 degrees, tripled to +-90).

use std::sync::atomic::{AtomicU32, Ordering};

/// Where the sounds are heard from: vanilla's camera (`ListenerTransform`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

impl Listener {
    /// `ListenerTransform.INITIAL`.
    pub const INITIAL: Listener = Listener {
        position: [0.0, 0.0, 0.0],
        forward: [0.0, 0.0, -1.0],
        up: [0.0, 1.0, 0.0],
    };

    /// Vanilla's camera at `position` turned to `y_rot` and `x_rot` degrees
    /// (`Camera.setRotation`: forward is the look vector, up is perpendicular to it).
    pub fn camera(position: [f64; 3], y_rot: f32, x_rot: f32) -> Listener {
        let (yaw, pitch) = (y_rot.to_radians(), x_rot.to_radians());
        let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
        Listener {
            position: position.map(|v| v as f32),
            forward: [-sy * cp, -sp, cy * cp],
            up: [-sy * sp, cp, cy * sp],
        }
    }

    /// A world position in the listener's space: x right, y up, z behind
    /// (`CalcContextParams`' matrix).
    pub fn to_local(&self, p: [f32; 3]) -> [f32; 3] {
        let n = normalized(self.forward);
        let v = normalized(self.up);
        let u = normalized(cross(n, v));
        let d = [
            p[0] - self.position[0],
            p[1] - self.position[1],
            p[2] - self.position[2],
        ];
        [dot(d, u), dot(d, v), -dot(d, n)]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalized(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 0.0 { a.map(|v| v / l) } else { a }
}

/// `AL_LINEAR_DISTANCE_CLAMPED` with a reference distance of 0 and a rolloff of 1.
pub fn linear_gain(distance: f32, max_distance: f32) -> f32 {
    if max_distance == 0.0 {
        // OpenAL Soft leaves the gain alone when the two distances are equal.
        return 1.0;
    }
    if max_distance.is_nan() || max_distance < 0.0 {
        return 1.0;
    }
    let d = distance.clamp(0.0, max_distance);
    (1.0 - d / max_distance).max(0.0)
}

/// `ScaleAzimuthFront3_2`: angles in front scaled by 1.5, clamped at 90 degrees.
pub fn scale_front_3_2(mut p: [f32; 3]) -> [f32; 3] {
    if p[2] < 0.0 {
        let len2d = (p[0] * p[0] + p[2] * p[2]).sqrt();
        let mut z = -p[2] / len2d;
        if z > 0.5 {
            let mut x = p[0] / len2d;
            // Halve the angle, then triple it.
            x = ((1.0 - z) * 0.5).sqrt().copysign(x);
            z = ((1.0 + z) * 0.5).sqrt();
            x = x * 3.0 - x * x * x * 4.0;
            z = z * z * z * 4.0 - z * 3.0;
            p[0] = x * len2d;
            p[2] = -z * len2d;
        } else {
            p[0] = len2d.copysign(p[0]);
            p[2] = 0.0;
        }
    }
    p
}

/// `ScaleAzimuthFront3`: angles in front tripled, clamped at 90 degrees.
pub fn scale_front_3(mut p: [f32; 3]) -> [f32; 3] {
    if p[2] < 0.0 {
        let len2d = (p[0] * p[0] + p[2] * p[2]).sqrt();
        let mut z = -p[2] / len2d;
        if z > 0.866_025_4 {
            let mut x = p[0] / len2d;
            x = x * 3.0 - x * x * x * 4.0;
            z = z * z * z * 4.0 - z * 3.0;
            p[0] = x * len2d;
            p[2] = -z * len2d;
        } else {
            p[0] = len2d.copysign(p[0]);
            p[2] = 0.0;
        }
    }
    p
}

/// The stereo decoder's gains for a direction (unit length, listener space).
pub fn decode(dir: [f32; 3]) -> [f32; 2] {
    const SQRT3: f32 = 1.732_050_8;
    // N3D first order, in OpenAL Soft's mapping from its coordinates.
    let (w, y, x) = (1.0, SQRT3 * -dir[0], SQRT3 * -dir[2]);
    [
        0.5 * w + 0.288_675_13 * y + 0.055_230_564 * x,
        0.5 * w - 0.288_675_13 * y + 0.055_230_564 * x,
    ]
}

/// How a mono source sounds: its distance gain and its pan, in the listener's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The position: in the world, or (relative sources) already in the listener's space.
    pub position: [f32; 3],
    pub relative: bool,
    /// The linear model's maximum distance, or none for no distance model.
    pub max_distance: Option<f32>,
}

/// The left and right gains of a mono source of gain `gain` (already clamped to 0..1).
pub fn mono_gains(listener: &Listener, place: &Placement, gain: f32) -> [f32; 2] {
    let local = if place.relative {
        place.position
    } else {
        listener.to_local(place.position)
    };
    let distance = length(local);
    let attenuation = place
        .max_distance
        .map_or(1.0, |max| linear_gain(distance, max));
    let g = (gain * attenuation).clamp(0.0, 1.0);
    let pan = if distance > f32::EPSILON {
        decode(scale_front_3_2(local.map(|v| v / distance)))
    } else {
        // At the listener: OpenAL Soft's mono channel, straight ahead.
        decode(scale_front_3([0.0, 0.0, -1.0]))
    };
    [g * pan[0], g * pan[1]]
}

/// A seqlock holding the listener: written by one thread every frame, read by the mixer
/// and the engine without locking.
#[derive(Debug)]
pub struct ListenerCell {
    seq: AtomicU32,
    values: [AtomicU32; 9],
}

impl Default for ListenerCell {
    fn default() -> ListenerCell {
        let cell = ListenerCell {
            seq: AtomicU32::new(0),
            values: Default::default(),
        };
        cell.store(&Listener::INITIAL);
        cell
    }
}

impl ListenerCell {
    /// Only ever called from one thread at a time.
    pub fn store(&self, l: &Listener) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        let all = [l.position, l.forward, l.up];
        for (i, v) in all.iter().flatten().enumerate() {
            self.values[i].store(v.to_bits(), Ordering::Relaxed);
        }
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// The latest listener, or `None` if a write kept getting in the way.
    pub fn load(&self) -> Option<Listener> {
        for _ in 0..64 {
            let before = self.seq.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let v: [f32; 9] =
                std::array::from_fn(|i| f32::from_bits(self.values[i].load(Ordering::Relaxed)));
            std::sync::atomic::fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == before {
                return Some(Listener {
                    position: [v[0], v[1], v[2]],
                    forward: [v[3], v[4], v[5]],
                    up: [v[6], v[7], v[8]],
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
    }

    const FRONT: f32 = 0.5 + 0.095_662_1;
    const BACK: f32 = 0.5 - 0.095_662_1;

    #[test]
    fn the_camera_faces_as_vanillas_does() {
        // Facing south (+z), east (+x) is on the left.
        let l = Listener::camera([0.0, 64.0, 0.0], 0.0, 0.0);
        assert!(close([l.forward[2], l.up[1]], [1.0, 1.0]));
        let east = l.to_local([1.0, 64.0, 0.0]);
        assert!((east[0] + 1.0).abs() < 1e-6, "{east:?}");
        // Facing east (yaw -90): south is on the right.
        let l = Listener::camera([0.0, 64.0, 0.0], -90.0, 0.0);
        let south = l.to_local([0.0, 64.0, 5.0]);
        assert!(
            (south[0] - 5.0).abs() < 1e-5 && south[2].abs() < 1e-5,
            "{south:?}"
        );
        // Looking straight down, what is below is ahead.
        let l = Listener::camera([0.0, 64.0, 0.0], 30.0, 90.0);
        let below = l.to_local([0.0, 60.0, 0.0]);
        assert!((below[2] + 4.0).abs() < 1e-5, "{below:?}");
    }

    #[test]
    fn linear_clamped_distance() {
        assert_eq!(linear_gain(0.0, 16.0), 1.0);
        assert_eq!(linear_gain(4.0, 16.0), 0.75);
        assert_eq!(linear_gain(16.0, 16.0), 0.0);
        assert_eq!(linear_gain(40.0, 16.0), 0.0);
        assert_eq!(linear_gain(3.0, 0.0), 1.0);
    }

    #[test]
    fn openal_softs_stereo_pan() {
        let l = Listener::INITIAL;
        let at = |p: [f32; 3]| {
            mono_gains(
                &l,
                &Placement {
                    position: p,
                    relative: false,
                    max_distance: None,
                },
                1.0,
            )
        };
        assert!(close(at([-3.0, 0.0, 0.0]), [1.0, 0.0]));
        assert!(close(at([3.0, 0.0, 0.0]), [0.0, 1.0]));
        assert!(close(at([0.0, 0.0, -2.0]), [FRONT, FRONT]));
        assert!(close(at([0.0, 0.0, 2.0]), [BACK, BACK]));
        assert!(close(at([0.0, 5.0, 0.0]), [0.5, 0.5]));
        assert!(close(at([0.0, 0.0, 0.0]), [FRONT, FRONT]));
        // 60 degrees to the right in front is already hard right; so is 80.
        let s = 60f32.to_radians().sin();
        let c = 60f32.to_radians().cos();
        assert!(close(at([s, 0.0, -c]), [0.0, 1.0]));
        // 30 degrees right becomes 45: left = 0.5 - 0.5 sin45 + 0.0957 cos45.
        let (s, c) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let want = [
            0.5 - 0.5 * h + 0.095_662_1 * h,
            0.5 + 0.5 * h + 0.095_662_1 * h,
        ];
        assert!(close(at([s, 0.0, -c]), want), "{:?}", at([s, 0.0, -c]));
        // Behind, the angle is kept: 135 degrees to the left.
        let want = [
            0.5 + 0.5 * h - 0.095_662_1 * h,
            0.5 - 0.5 * h - 0.095_662_1 * h,
        ];
        assert!(close(at([-1.0, 0.0, 1.0]), want));
    }

    #[test]
    fn distance_scales_both_ears() {
        let l = Listener::INITIAL;
        let place = Placement {
            position: [-4.0, 0.0, 0.0],
            relative: false,
            max_distance: Some(16.0),
        };
        assert!(close(mono_gains(&l, &place, 0.8), [0.8 * 0.75, 0.0]));
        let rel = Placement {
            relative: true,
            ..place
        };
        let moved = Listener {
            position: [100.0, 0.0, 0.0],
            ..l
        };
        assert!(close(mono_gains(&moved, &rel, 0.8), [0.8 * 0.75, 0.0]));
    }

    #[test]
    fn the_cell_hands_over_whole_listeners() {
        let cell = std::sync::Arc::new(ListenerCell::default());
        assert_eq!(cell.load(), Some(Listener::INITIAL));
        let writer = {
            let cell = cell.clone();
            std::thread::spawn(move || {
                for i in 0..20_000 {
                    let v = i as f32;
                    cell.store(&Listener {
                        position: [v; 3],
                        forward: [v; 3],
                        up: [v; 3],
                    });
                }
            })
        };
        for _ in 0..20_000 {
            if let Some(l) = cell.load()
                && l != Listener::INITIAL
            {
                let v = l.position[0];
                assert!(
                    [l.position, l.forward, l.up]
                        .iter()
                        .flatten()
                        .all(|&x| x == v)
                );
            }
        }
        writer.join().unwrap();
    }
}
