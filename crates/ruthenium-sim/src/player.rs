//! The local player, as far as the game thread keeps it so far: position, motion and
//! rotation, set by the server's teleports (`ClientPacketListener.handleMovePlayer`).
//! Physics and movement packets come next (PLAN Stage 2).

use ruthenium_javamath::mth;
use ruthenium_protocol::packets::play::PlayerPosition;

/// `Relative` bits.
const X: u32 = 1 << 0;
const Y: u32 = 1 << 1;
const Z: u32 = 1 << 2;
const Y_ROT: u32 = 1 << 3;
const X_ROT: u32 = 1 << 4;
const DELTA_X: u32 = 1 << 5;
const DELTA_Y: u32 = 1 << 6;
const DELTA_Z: u32 = 1 << 7;
const ROTATE_DELTA: u32 = 1 << 8;

/// Java's `Math.toRadians` constant.
const DEGREES_TO_RADIANS: f64 = 0.017453292519943295;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalPlayer {
    pub position: [f64; 3],
    pub delta: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
}

impl LocalPlayer {
    /// A new player (`handleLogin` turns it to face -180).
    pub fn new() -> LocalPlayer {
        LocalPlayer {
            position: [0.0; 3],
            delta: [0.0; 3],
            y_rot: -180.0,
            x_rot: 0.0,
        }
    }

    /// Applies a teleport (`PositionMoveRotation.calculateAbsolute`, then set without
    /// interpolation).
    pub fn teleport(&mut self, t: &PlayerPosition) {
        let rel = |bit: u32| t.relatives & bit != 0;
        let off = |bit: u32, v: f64| if rel(bit) { v } else { 0.0 };
        let position = [
            off(X, self.position[0]) + t.position[0],
            off(Y, self.position[1]) + t.position[1],
            off(Z, self.position[2]) + t.position[2],
        ];
        let y_rot = (if rel(Y_ROT) { self.y_rot } else { 0.0 }) + t.y_rot;
        let x_rot = ((if rel(X_ROT) { self.x_rot } else { 0.0 }) + t.x_rot).clamp(-90.0, 90.0);
        let mut moving = self.delta;
        if rel(ROTATE_DELTA) {
            let dy = self.y_rot - y_rot;
            let dx = self.x_rot - x_rot;
            moving = rotate_x(moving, (f64::from(dx) * DEGREES_TO_RADIANS) as f32);
            moving = rotate_y(moving, (f64::from(dy) * DEGREES_TO_RADIANS) as f32);
        }
        let delta = |cur: f64, change: f64, bit: u32| if rel(bit) { cur + change } else { change };
        self.delta = [
            delta(moving[0], t.delta[0], DELTA_X),
            delta(moving[1], t.delta[1], DELTA_Y),
            delta(moving[2], t.delta[2], DELTA_Z),
        ];
        self.position = position;
        self.y_rot = y_rot;
        self.x_rot = x_rot;
    }

    /// The chunk the camera (the player's block position) is in.
    pub fn chunk(&self) -> (i32, i32) {
        (
            mth::floor(self.position[0]) >> 4,
            mth::floor(self.position[2]) >> 4,
        )
    }
}

impl Default for LocalPlayer {
    fn default() -> LocalPlayer {
        LocalPlayer::new()
    }
}

/// `Vec3.xRot`: float sine and cosine from vanilla's table, products in doubles.
fn rotate_x(v: [f64; 3], radians: f32) -> [f64; 3] {
    let cos = f64::from(mth::cos(f64::from(radians)));
    let sin = f64::from(mth::sin(f64::from(radians)));
    [v[0], v[1] * cos + v[2] * sin, v[2] * cos - v[1] * sin]
}

/// `Vec3.yRot`.
fn rotate_y(v: [f64; 3], radians: f32) -> [f64; 3] {
    let cos = f64::from(mth::cos(f64::from(radians)));
    let sin = f64::from(mth::sin(f64::from(radians)));
    [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: [f64; 3], relatives: u32) -> PlayerPosition {
        PlayerPosition {
            id: 1,
            position,
            delta: [0.0; 3],
            y_rot: 90.0,
            x_rot: 100.0,
            relatives,
        }
    }

    #[test]
    fn absolute_and_relative_teleports() {
        let mut p = LocalPlayer::new();
        p.teleport(&at([1.5, -60.0, 9.5], 0));
        assert_eq!(p.position, [1.5, -60.0, 9.5]);
        // Pitch is clamped.
        assert_eq!((p.y_rot, p.x_rot), (90.0, 90.0));
        p.teleport(&at([1.0, 0.0, -1.0], X | Y | Z | Y_ROT));
        assert_eq!(p.position, [2.5, -60.0, 8.5]);
        assert_eq!(p.y_rot, 180.0);
        assert_eq!(p.chunk(), (0, 0));
        p.teleport(&at([-0.5, 0.0, -16.0], 0));
        assert_eq!(p.chunk(), (-1, -1));
    }
}
