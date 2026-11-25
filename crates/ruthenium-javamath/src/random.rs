//! `java.util.Random`'s generator, which vanilla's `LegacyRandomSource` is: a 48-bit
//! linear congruential generator.

const MULTIPLIER: i64 = 0x5_DEEC_E66D;
const INCREMENT: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;

/// `java.util.Random`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JavaRandom(i64);

impl JavaRandom {
    pub fn new(seed: i64) -> JavaRandom {
        JavaRandom((seed ^ MULTIPLIER) & MASK)
    }

    /// `setSeed`.
    pub fn set_seed(&mut self, seed: i64) {
        *self = JavaRandom::new(seed);
    }

    /// `next(bits)`.
    pub fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT)) & MASK;
        (self.0 >> (48 - bits)) as i32
    }

    /// `nextInt(bound)`, for a bound above 0.
    pub fn next_int(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0);
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let value = bits % bound;
            if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
                return value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_what_java_draws() {
        // new Random(42): nextInt(2) x 4, nextInt(3) x 4, nextInt(10) x 4.
        let mut r = JavaRandom::new(42);
        let two: Vec<i32> = (0..4).map(|_| r.next_int(2)).collect();
        let three: Vec<i32> = (0..4).map(|_| r.next_int(3)).collect();
        let ten: Vec<i32> = (0..4).map(|_| r.next_int(10)).collect();
        assert_eq!(
            (two, three, ten),
            (JAVA.0.to_vec(), JAVA.1.to_vec(), JAVA.2.to_vec())
        );
        r.set_seed(42);
        assert_eq!(r, JavaRandom::new(42));
    }

    /// Printed by the JVM.
    const JAVA: ([i32; 4], [i32; 4], [i32; 4]) = ([1, 0, 1, 0], [0, 1, 2, 2], [9, 3, 2, 2]);
}
