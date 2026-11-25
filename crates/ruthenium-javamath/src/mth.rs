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

/// `Mth.FRAC_BIAS`: added to a number from 0 to 1 it leaves the number's first eight
/// binary places in the low bits of the sum.
const FRAC_BIAS: f64 = f64::from_bits(4805340802404319232);

/// The entries of `Mth.COS_TAB` where the game's `Math.cos` (HotSpot's intrinsic on
/// x86-64, OpenJDK 25) gives the neighbour of what fdlibm's algorithm gives: the index
/// and the bits of the JVM's value.
const COS_TAB_JVM: [(usize, u64); 8] = [
    (125, 0x3febed0bdee7064d),
    (127, 0x3febc8dbfdbfda88),
    (141, 0x3feab5732734fa47),
    (170, 0x3fe7ecf874b086df),
    (175, 0x3fe75b090e40447a),
    (181, 0x3fe6a13cc8a9b946),
    (182, 0x3fe681110a985d4d),
    (186, 0x3fe5fcb9f031317b),
];

/// `Mth.ASIN_TAB` and `Mth.COS_TAB`: the arcsine of each 256th from 0 to 1, and its
/// cosine.
static ATAN2_TABLES: LazyLock<([f64; 257], [f64; 257])> = LazyLock::new(|| {
    let (mut asin, mut cos) = ([0.0; 257], [0.0; 257]);
    for i in 0..257 {
        let a = libm::asin(i as f64 / 256.0);
        asin[i] = a;
        cos[i] = libm::cos(a);
    }
    for (i, bits) in COS_TAB_JVM {
        cos[i] = f64::from_bits(bits);
    }
    (asin, cos)
});

/// Vanilla `Mth.fastInvSqrt(double)`: one step of Newton's method from a guess made of
/// the number's bits.
#[inline]
pub fn fast_inv_sqrt(x: f64) -> f64 {
    let half = 0.5 * x;
    let i = 6910469410427058090_i64.wrapping_sub((x.to_bits() as i64) >> 1);
    let x = f64::from_bits(i as u64);
    x * (1.5 - half * x * x)
}

/// Vanilla `Mth.atan2(double, double)`: the angle of the point (x, y), from a table of
/// arcsines and one correction step. Not `Math.atan2`: it is off by up to about 1e-10
/// and the game's results depend on which.
pub fn atan2(mut y: f64, mut x: f64) -> f64 {
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let neg_y = y < 0.0;
    if neg_y {
        y = -y;
    }
    let neg_x = x < 0.0;
    if neg_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    let rinv = fast_inv_sqrt(d2);
    x *= rinv;
    y *= rinv;
    let yp = FRAC_BIAS + y;
    // `(int) Double.doubleToRawLongBits(yp)`: the low 32 bits. For every finite input
    // they are 0 to 256; anything else (both coordinates zero or infinite) has no
    // entry, where Java throws.
    let index = yp.to_bits() as u32 as i32;
    let Some(i) = usize::try_from(index).ok().filter(|i| *i < 257) else {
        return f64::NAN;
    };
    let (phi, c_phi) = (ATAN2_TABLES.0[i], ATAN2_TABLES.1[i]);
    let s_phi = yp - FRAC_BIAS;
    let sd = y * c_phi - x * s_phi;
    let d = (6.0 + sd * sd) * sd * 0.16666666666666666;
    let mut theta = phi + d;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if neg_x {
        theta = std::f64::consts::PI - theta;
    }
    if neg_y {
        theta = -theta;
    }
    theta
}

/// Java's `Math.cos(double)`, for the few places the game calls it rather than
/// `Mth.cos` (elytra lift). This is fdlibm's algorithm (`StrictMath.cos`); HotSpot's
/// intrinsic is allowed to differ from it by an ulp, which the golden traces would show.
#[inline]
pub fn java_cos(radians: f64) -> f64 {
    libm::cos(radians)
}

/// Java's `Math.sin(double)`, for the few places the game calls it rather than
/// `Mth.sin` (the elastic easing curves). fdlibm's algorithm, as [`java_cos`].
#[inline]
pub fn java_sin(radians: f64) -> f64 {
    libm::sin(radians)
}

/// Java's `Math.pow(double, double)` (the easing curves). fdlibm's algorithm; HotSpot's
/// intrinsic is allowed to differ from it by an ulp.
#[inline]
pub fn java_pow(base: f64, exponent: f64) -> f64 {
    libm::pow(base, exponent)
}

/// Java's `Math.min(float, float)`: NaN when either is, and -0 below +0.
#[inline]
pub fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        // Equal values differ only as zeros: the negative one is the lesser.
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// Java's `Math.max(float, float)`: NaN when either is, and +0 above -0.
#[inline]
pub fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
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
    fn min_and_max_order_the_zeros() {
        assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(java_min(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(java_max(0.0, -0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(java_max(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!((java_min(1.0, 2.0), java_max(1.0, 2.0)), (1.0, 2.0));
        assert!(java_min(f32::NAN, 1.0).is_nan() && java_max(1.0, f32::NAN).is_nan());
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
