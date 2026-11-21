//! Java-exact math for Ruthenium.
//!
//! Everything here must produce results bit-identical to the JVM running the
//! vanilla client: the 65536-entry sine table, Java's float-to-integer
//! conversions, `floor`. Rust already matches the JVM on the parts that matter:
//! IEEE-754 `+ - * /` with round-to-nearest-even, no implicit FMA contraction,
//! and saturating `as` casts with NaN mapping to 0 (the same as Java's `d2i` and
//! `d2l`). Never use fused multiply-add or fast-math here.
//!
//! Licensed MIT OR Apache-2.0 so other projects can reuse it.

#![forbid(unsafe_code)]

pub mod mth;
