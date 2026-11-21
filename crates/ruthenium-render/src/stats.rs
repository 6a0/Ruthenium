//! Frame-time statistics over a sliding window, O(1) per frame and allocation-free after
//! construction.
//!
//! Definitions, as PresentMon and CapFrameX use them: "1% low" = 1000 / P99 frame time (ms),
//! "0.1% low" = 1000 / P99.9 frame time. Times are binned in buckets 1% wide (log scale) from
//! 1 µs to 10 s, so a reported percentile is within 0.5% of the true sample.

const MIN_NS: f64 = 1_000.0;
const RATIO: f64 = 1.01;
/// ln(10 s / 1 µs) / ln(1.01) ≈ 1620, plus the under- and overflow buckets.
const BUCKETS: usize = 1624;

fn bucket(ns: u64) -> usize {
    let ns = ns as f64;
    if ns < MIN_NS {
        return 0;
    }
    (1 + ((ns / MIN_NS).ln() / RATIO.ln()) as usize).min(BUCKETS - 1)
}

/// Representative value of a bucket: its geometric middle.
fn bucket_ns(b: usize) -> f64 {
    if b == 0 {
        MIN_NS / 2.0
    } else {
        MIN_NS * RATIO.powf(b as f64 - 0.5)
    }
}

/// The last `capacity` samples of one series (frame interval, CPU time or GPU time).
#[derive(Debug)]
pub struct FrameTimes {
    counts: Box<[u32]>,
    ring: Box<[u64]>,
    head: usize,
    len: usize,
    sum_ns: u64,
    max_ns: u64,
}

/// Percentiles of a series at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub samples: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub p999_ms: f64,
    pub max_ms: f64,
}

impl Summary {
    /// Average rate, frames per second.
    pub fn fps(&self) -> f64 {
        if self.mean_ms > 0.0 {
            1000.0 / self.mean_ms
        } else {
            0.0
        }
    }

    pub fn low_1_percent_fps(&self) -> f64 {
        if self.p99_ms > 0.0 {
            1000.0 / self.p99_ms
        } else {
            0.0
        }
    }

    pub fn low_01_percent_fps(&self) -> f64 {
        if self.p999_ms > 0.0 {
            1000.0 / self.p999_ms
        } else {
            0.0
        }
    }
}

impl FrameTimes {
    /// Keeps the last `capacity` samples (at least 1).
    pub fn new(capacity: usize) -> FrameTimes {
        FrameTimes {
            counts: vec![0; BUCKETS].into_boxed_slice(),
            ring: vec![0; capacity.max(1)].into_boxed_slice(),
            head: 0,
            len: 0,
            sum_ns: 0,
            max_ns: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.counts.fill(0);
        self.head = 0;
        self.len = 0;
        self.sum_ns = 0;
        self.max_ns = 0;
    }

    pub fn push(&mut self, ns: u64) {
        if self.len == self.ring.len() {
            let old = self.ring[self.head];
            self.counts[bucket(old)] -= 1;
            self.sum_ns -= old;
        } else {
            self.len += 1;
        }
        self.ring[self.head] = ns;
        self.head = (self.head + 1) % self.ring.len();
        self.counts[bucket(ns)] += 1;
        self.sum_ns += ns;
        // The maximum is over everything since the last clear, not just the window.
        self.max_ns = self.max_ns.max(ns);
    }

    /// The `p`-quantile (0..=1) in milliseconds: the smallest bucket at or above rank
    /// ceil(p·n).
    pub fn percentile_ms(&self, p: f64) -> f64 {
        if self.len == 0 {
            return 0.0;
        }
        let rank = ((p.clamp(0.0, 1.0) * self.len as f64).ceil() as usize).max(1);
        let mut seen = 0usize;
        for (b, &c) in self.counts.iter().enumerate() {
            seen += c as usize;
            if seen >= rank {
                return bucket_ns(b) / 1e6;
            }
        }
        bucket_ns(BUCKETS - 1) / 1e6
    }

    pub fn summary(&self) -> Summary {
        if self.len == 0 {
            return Summary::default();
        }
        Summary {
            samples: self.len,
            mean_ms: self.sum_ns as f64 / self.len as f64 / 1e6,
            p50_ms: self.percentile_ms(0.5),
            p99_ms: self.percentile_ms(0.99),
            p999_ms: self.percentile_ms(0.999),
            max_ms: self.max_ns as f64 / 1e6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= b * 0.006
    }

    #[test]
    fn percentiles_of_a_known_distribution() {
        let mut t = FrameTimes::new(10_000);
        for _ in 0..1000 {
            t.push(1_000_000);
        }
        for _ in 0..10 {
            t.push(10_000_000);
        }
        let s = t.summary();
        assert_eq!(s.samples, 1010);
        // Rank ceil(0.99 * 1010) = 1000 is still a 1 ms frame; ceil(0.999 * 1010) = 1009 is not.
        assert!(close(s.p50_ms, 1.0), "{s:?}");
        assert!(close(s.p99_ms, 1.0), "{s:?}");
        assert!(close(s.p999_ms, 10.0), "{s:?}");
        assert!((s.mean_ms - (1000.0 + 100.0) / 1010.0).abs() < 1e-9);
        assert_eq!(s.max_ms, 10.0);
        assert!(close(s.low_01_percent_fps(), 100.0));
    }

    #[test]
    fn window_slides() {
        let mut t = FrameTimes::new(100);
        for _ in 0..100 {
            t.push(20_000_000);
        }
        for _ in 0..100 {
            t.push(2_000_000);
        }
        let s = t.summary();
        assert_eq!(s.samples, 100);
        assert!(close(s.p999_ms, 2.0), "{s:?}");
        assert!((s.mean_ms - 2.0).abs() < 1e-9);
    }

    #[test]
    fn extremes_are_clamped() {
        let mut t = FrameTimes::new(4);
        t.push(0);
        t.push(u64::from(u32::MAX) * 1000);
        assert!(t.percentile_ms(0.0) < 0.001);
        assert!(t.percentile_ms(1.0) >= 10_000.0);
        t.clear();
        assert_eq!(t.summary(), Summary::default());
    }
}
