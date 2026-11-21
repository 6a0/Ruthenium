//! Java-exact math for Ruthenium.
//!
//! Everything here must produce results bit-identical to the JVM: the 65536-entry
//! sine table, Java's rounding, `min`/`max` and NaN semantics. No FMA, no
//! fast-math. Each function is checked against values dumped from a real JVM.
//!
//! Licensed MIT OR Apache-2.0 so other projects can reuse it.

#![forbid(unsafe_code)]
