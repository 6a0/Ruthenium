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

/// Vanilla `Mth.lfloor(double)`: `(long) Math.floor(v)`, saturating, NaN to 0.
#[inline]
pub fn lfloor(v: f64) -> i64 {
    v.floor() as i64
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
    fn table_landmarks() {
        let t = sin_table();
        assert_eq!(t[0], 0.0);
        assert_eq!(t[16384], 1.0);
        assert_eq!(cos(0.0), 1.0);
    }
}
