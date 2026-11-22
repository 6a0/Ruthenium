//! Vanilla's `Mth` helpers (`net/minecraft/util/Mth.java`).
//!
//! Vanilla does not call `Math.sin` for gameplay. It looks values up in a
//! 65536-entry table of `f32`, so movement, rotation vectors and projectile
//! maths depend on the table's exact contents and on how inputs are turned into
//! indices. Both are reproduced here and tested against a dump from the JVM.

use std::sync::LazyLock;

/// Entries in the sine table: one full turn.
pub const SIN_TABLE_LEN: usize = 65536;

/// Table entries per radian, `65536 / 2π` as vanilla writes it (`Mth.java:34`).
pub const SIN_SCALE: f64 = 10430.378350470453;

/// Index offset of a quarter turn: `cos(x) = table[x * scale + 16384]`.
const QUARTER_TURN: f64 = 16384.0;

static SIN_TABLE: LazyLock<Box<[f32; SIN_TABLE_LEN]>> = LazyLock::new(|| {
    let mut table = vec![0.0f32; SIN_TABLE_LEN];
    for (i, slot) in table.iter_mut().enumerate() {
        // Vanilla: `(float) Math.sin(i / SIN_SCALE)` (`Mth.java:37`). `libm` is a
        // pure-Rust port of the fdlibm algorithms, so the result does not depend on
        // the platform C library.
        *slot = libm::sin(i as f64 / SIN_SCALE) as f32;
    }
    table
        .into_boxed_slice()
        .try_into()
        .expect("length is SIN_TABLE_LEN")
});

/// The whole sine table, built on first use.
pub fn sin_table() -> &'static [f32; SIN_TABLE_LEN] {
    &SIN_TABLE
}

/// Converts an already-scaled angle to a table index the way vanilla does:
/// `(int)((long) scaled & 65535L)`. The `as i64` cast saturates and maps NaN
/// to 0, exactly like Java's `(long)` cast.
#[inline]
fn index(scaled: f64) -> usize {
    ((scaled as i64) & (SIN_TABLE_LEN as i64 - 1)) as usize
}

/// Vanilla `Mth.sin(double)` (`Mth.java:50`). Callers passing `f32` widen it
/// to `f64` first, as Java does.
#[inline]
pub fn sin(radians: f64) -> f32 {
    SIN_TABLE[index(radians * SIN_SCALE)]
}

/// Vanilla `Mth.cos(double)` (`Mth.java:54`).
#[inline]
pub fn cos(radians: f64) -> f32 {
    SIN_TABLE[index(radians * SIN_SCALE + QUARTER_TURN)]
}

/// Vanilla `Mth.floor(float)`: `(int) Math.floor(v)`, saturating, NaN to 0.
#[inline]
pub fn floor_f32(v: f32) -> i32 {
    f64::from(v).floor() as i32
}

/// Vanilla `Mth.floor(double)`: `(int) Math.floor(v)`, saturating, NaN to 0.
#[inline]
pub fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// Vanilla `Mth.ceil(double)`: the int cast, plus one when the value is greater.
#[inline]
pub fn ceil(v: f64) -> i32 {
    let truncated = v as i32;
    if v > f64::from(truncated) {
        truncated + 1
    } else {
        truncated
    }
}

/// Vanilla `Mth.lfloor(double)`: `(long) Math.floor(v)`, saturating, NaN to 0.
#[inline]
pub fn lfloor(v: f64) -> i64 {
    v.floor() as i64
}

/// Java's `Math.round(double)`: to the nearest long, halves up (towards positive
/// infinity, where Rust's `round` goes away from zero), saturating, NaN to 0.
#[inline]
pub fn round(v: f64) -> i64 {
    let r = v.round();
    // Within half of each other, so the difference is exact.
    (if v - r == 0.5 { r + 1.0 } else { r }) as i64
}

/// Vanilla `Mth.wrapDegrees(float)`: into [-180, 180).
#[inline]
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// Vanilla `Mth.wrapDegrees(double)`.
#[inline]
pub fn wrap_degrees_f64(angle: f64) -> f64 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// Vanilla `Mth.lerp(float, float, float)`.
#[inline]
pub fn lerp(alpha: f32, from: f32, to: f32) -> f32 {
    from + alpha * (to - from)
}

/// Vanilla `Mth.lerp(double, double, double)`.
#[inline]
pub fn lerp_f64(alpha: f64, from: f64, to: f64) -> f64 {
    from + alpha * (to - from)
}

/// Vanilla `Mth.rotLerp(float, float, float)`: the shorter way round.
#[inline]
pub fn rot_lerp(alpha: f32, from: f32, to: f32) -> f32 {
    from + alpha * wrap_degrees(to - from)
}

/// Vanilla `Mth.rotLerp(double, double, double)`.
#[inline]
pub fn rot_lerp_f64(alpha: f64, from: f64, to: f64) -> f64 {
    from + alpha * wrap_degrees_f64(to - from)
}

/// Vanilla `Mth.unpackDegrees`: a rotation sent as a byte, 256 steps a turn.
#[inline]
pub fn unpack_degrees(packed: i8) -> f32 {
    (i32::from(packed) * 360) as f32 / 256.0
}

/// Vanilla `Mth.absMax(double, double)`.
#[inline]
pub fn abs_max(a: f64, b: f64) -> f64 {
    a.abs().max(b.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_follows_java_long_cast() {
        assert_eq!(index(f64::NAN), 0);
        assert_eq!(index(f64::INFINITY), (i64::MAX & 0xFFFF) as usize);
        assert_eq!(index(f64::NEG_INFINITY), 0); // i64::MIN & 0xFFFF
        assert_eq!(index(-1.0), 0xFFFF);
        assert_eq!(index(-0.5), 0); // truncation toward zero, not floor
        assert_eq!(index(65536.0 + 3.9), 3);
    }

    #[test]
    fn floor_follows_java_int_cast() {
        assert_eq!(floor(-0.5), -1);
        assert_eq!(floor(f64::NAN), 0);
        assert_eq!(floor(1e300), i32::MAX);
        assert_eq!(floor(-1e300), i32::MIN);
        assert_eq!(floor_f32(-1e-45), -1);
        assert_eq!(lfloor(-1e300), i64::MIN);
    }

    #[test]
    fn round_goes_half_up_like_java() {
        assert_eq!(round(2.5), 3);
        assert_eq!(round(-2.5), -2);
        assert_eq!(round(-0.5), 0);
        assert_eq!(round(0.49999999999999994), 0);
        assert_eq!(round(-1.5000000000000002), -2);
        assert_eq!(round(f64::NAN), 0);
        assert_eq!(round(1e300), i64::MAX);
    }

    #[test]
    fn degrees() {
        assert_eq!(wrap_degrees(180.0), -180.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees_f64(540.0), -180.0);
        assert_eq!(rot_lerp(0.5, 170.0, -170.0), 180.0);
        assert_eq!(unpack_degrees(-128), -180.0);
        assert_eq!(unpack_degrees(64), 90.0);
    }

    #[test]
    fn table_landmarks() {
        let t = sin_table();
        assert_eq!(t[0], 0.0);
        assert_eq!(t[16384], 1.0);
        assert_eq!(cos(0.0), 1.0);
    }
}
