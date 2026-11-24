//! How many bytes the render thread may copy into staging, by phase.
//!
//! - **Loading** (start-up, a dimension switch, a blocking pack reload): a burst may take
//!   whatever the ring has free, and the next burst goes as soon as a submission has made
//!   room. The loading screen's deadlines are the caller's to keep.
//! - **Gameplay**: a ceiling per frame. It starts as the fixed byte count the ring was
//!   created with; once the frame loop reports its pace ([`Pacing::frame`]), it is the
//!   smaller of that and a time-derived one: a fraction of the frame interval, turned
//!   into bytes with the measured speed of writes into the mapped ring, under a separate
//!   cap on sustained bytes per second.
//!
//! The numbers here are starting values to tune against real terrain streaming.

use std::time::Duration;

/// The phase the staging budget is set for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UploadPhase {
    /// Behind a loading screen: bursts up to the ring's free space.
    Loading,
    /// While playing: a ceiling per frame.
    #[default]
    Gameplay,
}

/// The time-derived ceiling's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PacingConfig {
    /// The part of a frame interval staging may take (0.05 to 0.10).
    pub fraction: f64,
    /// The ceiling is never below or above these, whatever the interval.
    pub min: Duration,
    pub max: Duration,
    /// Bytes per second staging may sustain; a quarter of a second of it may go at once.
    pub sustained_bytes_per_s: u64,
    /// Bytes per second written into the mapped ring, until writes have been measured.
    pub write_bytes_per_s: f64,
}

impl PacingConfig {
    /// Starting values for a GPU with memory of its own: a tenth of the interval.
    pub const DISCRETE: PacingConfig = PacingConfig {
        fraction: 0.10,
        min: Duration::from_micros(48),
        max: Duration::from_micros(500),
        sustained_bytes_per_s: 96 << 20,
        write_bytes_per_s: 2e9,
    };
    /// Starting values where the CPU and the GPU share memory bandwidth: a twentieth.
    pub const INTEGRATED: PacingConfig = PacingConfig {
        fraction: 0.05,
        sustained_bytes_per_s: 48 << 20,
        ..PacingConfig::DISCRETE
    };
}

/// Frame intervals the smoothed interval looks back over.
const WINDOW: usize = 8;
/// Writes shorter than this are not used to measure the write speed: too noisy.
const MEASURED_WRITE: u64 = 64 << 10;

/// The time-derived ceiling of the gameplay phase.
///
/// The interval it works from is the smaller of the target interval and a smoothed recent
/// one, and the smoothed one follows the *shortest* of the last few intervals: a single
/// long frame (a stall) is not the shortest of anything, so it never raises the next
/// frame's allowance, while a pace that really has become slower raises it once every
/// interval of the window is long.
#[derive(Clone, Copy, Debug)]
pub struct Pacing {
    config: PacingConfig,
    recent: [u64; WINDOW],
    count: usize,
    /// The smoothed interval, in nanoseconds (0: no frame yet).
    smoothed_ns: f64,
    write_bytes_per_s: f64,
    /// Bytes the sustained cap still allows.
    tokens: f64,
    /// The time ceiling and the bytes computed for the last frame.
    ceiling: Duration,
    allowance: u64,
}

impl Pacing {
    pub fn new(config: PacingConfig) -> Pacing {
        assert!(
            config.fraction > 0.0 && config.fraction <= 1.0 && config.min <= config.max,
            "inconsistent pacing: {config:?}"
        );
        Pacing {
            config,
            recent: [0; WINDOW],
            count: 0,
            smoothed_ns: 0.0,
            write_bytes_per_s: config.write_bytes_per_s,
            tokens: Pacing::burst(&config),
            ceiling: config.min,
            allowance: 0,
        }
    }

    fn burst(config: &PacingConfig) -> f64 {
        config.sustained_bytes_per_s as f64 / 4.0
    }

    /// The bytes the frame that starts now may stage. `interval` is the time since the
    /// frame before it started, `target` the interval the frame loop aims for if it has
    /// one (VSync, a frame limiter), `time_left` what remains before this frame's deadline
    /// if that is known: with less left than the ceiling, less is admitted.
    pub fn frame(
        &mut self,
        interval: Duration,
        target: Option<Duration>,
        time_left: Option<Duration>,
    ) -> u64 {
        let ns = interval.as_nanos().min(u128::from(u64::MAX)) as u64;
        self.recent[self.count % WINDOW] = ns;
        self.count += 1;
        let seen = self.count.min(WINDOW);
        let shortest = self.recent[..seen].iter().copied().min().unwrap_or(ns) as f64;
        self.smoothed_ns = if self.smoothed_ns == 0.0 || shortest < self.smoothed_ns {
            shortest
        } else {
            self.smoothed_ns + (shortest - self.smoothed_ns) / 4.0
        };
        let mut basis = self.smoothed_ns;
        if let Some(t) = target {
            basis = basis.min(t.as_nanos() as f64);
        }
        let ceiling = Duration::from_nanos((basis * self.config.fraction) as u64)
            .clamp(self.config.min, self.config.max);
        self.ceiling = match time_left {
            Some(left) => ceiling.min(left),
            None => ceiling,
        };
        // The sustained cap fills at the smoothed pace, not for the time a stall took.
        let filled = (ns as f64).min(self.smoothed_ns) / 1e9;
        self.tokens = (self.tokens + filled * self.config.sustained_bytes_per_s as f64)
            .min(Pacing::burst(&self.config));
        let by_time = self.ceiling.as_secs_f64() * self.write_bytes_per_s;
        self.allowance = by_time.min(self.tokens).max(0.0) as u64;
        self.allowance
    }

    /// `bytes` were staged: they count against the sustained cap.
    pub fn spent(&mut self, bytes: u64) {
        self.tokens -= bytes as f64;
    }

    /// A write of `bytes` into the mapped ring took `took`: a sample of its speed.
    pub fn wrote(&mut self, bytes: u64, took: Duration) {
        if bytes < MEASURED_WRITE || took.is_zero() {
            return;
        }
        let speed = bytes as f64 / took.as_secs_f64();
        self.write_bytes_per_s += (speed - self.write_bytes_per_s) / 8.0;
    }

    /// The time ceiling of the last frame.
    pub fn ceiling(&self) -> Duration {
        self.ceiling
    }

    /// The bytes the last frame was allowed.
    pub fn allowance(&self) -> u64 {
        self.allowance
    }

    /// The measured speed of writes into the mapped ring, in bytes per second.
    pub fn write_bytes_per_s(&self) -> f64 {
        self.write_bytes_per_s
    }
}

/// Frames a refused large upload keeps its claim without retrying.
const WAITING_FRAMES: u64 = 2;

/// The per-frame budget's fairness rule. An upload larger than the budget only goes in a
/// frame with nothing else staged, so a steady stream of small uploads (staged right after
/// each frame records) could starve it forever. Once such an upload is refused, smaller
/// ones are refused too unless both would fit, until one at least that large goes; a claim
/// its caller stops retrying lapses after [`WAITING_FRAMES`].
#[derive(Debug, Default, Clone, Copy)]
struct Waiting {
    /// Size of the largest refused upload, and the frame it was last refused in.
    claim: Option<(u64, u64)>,
}

impl Waiting {
    /// Whether `bytes` more may be staged now, with `staged` already staged for `frame`.
    fn admit(&mut self, staged: u64, bytes: u64, budget: u64, frame: u64) -> bool {
        if let Some((_, at)) = self.claim
            && frame > at + WAITING_FRAMES
        {
            self.claim = None;
        }
        let claimed = self.claim.map_or(0, |(size, _)| size);
        let fits = if bytes < claimed {
            staged + bytes + claimed <= budget
        } else {
            staged == 0 || staged + bytes <= budget
        };
        if fits {
            if bytes >= claimed {
                self.claim = None;
            }
        } else if bytes >= claimed {
            self.claim = Some((bytes, frame));
        }
        fits
    }
}

/// The staging budget of the upload ring: the phase, the fixed and the time-derived
/// ceiling, and what has been staged against them.
#[derive(Debug)]
pub(crate) struct Budget {
    phase: UploadPhase,
    /// Bytes one frame may stage in the gameplay phase, at most.
    fixed: u64,
    pub(crate) pacing: Pacing,
    /// The time-derived allowance of the frame being staged, if its pace was reported.
    paced: Option<u64>,
    /// Bytes staged since the last frame was recorded (in the loading phase: since the
    /// last submission).
    staged: u64,
    waiting: Waiting,
    /// Uploads refused because the frame's allowance was used up.
    pub(crate) refused: u64,
}

impl Budget {
    pub(crate) fn new(fixed: u64, pacing: PacingConfig) -> Budget {
        Budget {
            phase: UploadPhase::default(),
            fixed,
            pacing: Pacing::new(pacing),
            paced: None,
            staged: 0,
            waiting: Waiting::default(),
            refused: 0,
        }
    }

    pub(crate) fn phase(&self) -> UploadPhase {
        self.phase
    }

    pub(crate) fn set_phase(&mut self, phase: UploadPhase) {
        self.phase = phase;
        self.waiting = Waiting::default();
    }

    pub(crate) fn fixed(&self) -> u64 {
        self.fixed
    }

    pub(crate) fn set_fixed(&mut self, bytes: u64) {
        self.fixed = bytes;
    }

    pub(crate) fn staged(&self) -> u64 {
        self.staged
    }

    /// The frame loop's pace for the frame being staged (gameplay).
    pub(crate) fn pace(
        &mut self,
        interval: Duration,
        target: Option<Duration>,
        time_left: Option<Duration>,
    ) {
        self.paced = Some(self.pacing.frame(interval, target, time_left));
    }

    /// The bytes that may be staged between two frames (gameplay) or two submissions
    /// (loading).
    pub(crate) fn allowance(&self) -> u64 {
        match self.phase {
            UploadPhase::Loading => u64::MAX,
            UploadPhase::Gameplay => self.paced.map_or(self.fixed, |p| p.min(self.fixed)),
        }
    }

    /// What is left of the allowance.
    pub(crate) fn remaining(&self) -> u64 {
        self.allowance().saturating_sub(self.staged)
    }

    /// Whether `bytes` more may be staged for `frame`. Anything goes when nothing else is
    /// staged, so an upload larger than the allowance still goes, alone; see [`Waiting`]
    /// for how it gets that empty frame.
    pub(crate) fn admit(&mut self, bytes: u64, frame: u64) -> bool {
        let allowance = self.allowance();
        let fits =
            allowance == u64::MAX || self.waiting.admit(self.staged, bytes, allowance, frame);
        if !fits {
            self.refused += 1;
        }
        fits
    }

    /// `bytes` were staged.
    pub(crate) fn spend(&mut self, bytes: u64) {
        self.staged += bytes;
        if self.phase == UploadPhase::Gameplay {
            self.pacing.spent(bytes);
        }
    }

    /// A frame recorded what was staged: the next frame's allowance starts. Its pace has
    /// to be reported again, or the fixed ceiling applies.
    pub(crate) fn frame_recorded(&mut self) {
        self.staged = 0;
        self.paced = None;
    }

    /// A submission between frames took what was staged. Only a loading phase gets a new
    /// burst for it: in gameplay the ceiling is per frame, however many submissions go.
    pub(crate) fn submitted(&mut self) {
        if self.phase == UploadPhase::Loading {
            self.staged = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_large_upload_is_not_starved_by_small_ones() {
        let mut w = Waiting::default();
        let budget = 100;
        // Frame 1: small uploads staged first; the large one is refused and claims.
        assert!(w.admit(0, 10, budget, 1));
        assert!(!w.admit(10, 400, budget, 1));
        // Frame 2: small uploads now wait (10 + 400 > 100) ...
        assert!(!w.admit(0, 10, budget, 2));
        // ... so the large one finds the frame empty and goes alone.
        assert!(w.admit(0, 400, budget, 2));
        // Then small ones go again.
        assert!(w.admit(0, 10, budget, 3));
        // A claim nobody retries lapses.
        assert!(!w.admit(10, 400, budget, 4));
        assert!(!w.admit(0, 10, budget, 5));
        assert!(w.admit(0, 10, budget, 4 + WAITING_FRAMES + 1));
        // Small uploads that fit beside a claim under the budget still go.
        let mut w = Waiting::default();
        assert!(!w.admit(60, 50, budget, 1));
        assert!(w.admit(0, 10, budget, 2));
        assert!(w.admit(10, 50, budget, 2));
    }

    #[test]
    fn the_ceiling_is_a_clamped_fraction_of_the_interval() {
        let mut p = Pacing::new(PacingConfig::DISCRETE);
        // About 2,080 FPS: a tenth of 0.48 ms is the 48 microsecond floor.
        p.frame(Duration::from_micros(480), None, None);
        assert_eq!(p.ceiling(), Duration::from_micros(48));
        // 60 Hz: a tenth would be 1.67 ms; the ceiling stops at 500 microseconds.
        let mut p = Pacing::new(PacingConfig::DISCRETE);
        p.frame(Duration::from_micros(16_667), None, None);
        assert_eq!(p.ceiling(), Duration::from_micros(500));
        // The target interval counts when it is the shorter one.
        let mut p = Pacing::new(PacingConfig::INTEGRATED);
        p.frame(20 * MS, Some(4 * MS), None);
        assert_eq!(p.ceiling(), Duration::from_micros(200));
        // Bytes follow from the write speed: 200 microseconds at 2 GB/s.
        assert!(
            (399_999..=400_001).contains(&p.allowance()),
            "{}",
            p.allowance()
        );
    }

    #[test]
    fn a_stall_never_raises_the_next_allowance() {
        let mut p = Pacing::new(PacingConfig {
            sustained_bytes_per_s: 1 << 40,
            ..PacingConfig::DISCRETE
        });
        let mut steady = 0;
        for _ in 0..32 {
            steady = p.frame(2 * MS, None, None);
        }
        assert_eq!(p.ceiling(), Duration::from_micros(200));
        // One frame takes 300 ms, then several: each next allowance is no larger.
        for stall in [300 * MS, 40 * MS, 900 * MS] {
            let after = p.frame(stall, None, None);
            assert!(
                after <= steady,
                "{after} > {steady} after a {stall:?} frame"
            );
            let next = p.frame(2 * MS, None, None);
            assert!(next <= steady, "{next} > {steady} after a {stall:?} frame");
        }
        // Stalls every other frame are still stalls.
        for i in 0..40 {
            let interval = if i % 2 == 0 { 100 * MS } else { 2 * MS };
            assert!(p.frame(interval, None, None) <= steady);
        }
        // A pace that really is slower, every frame of the window, does raise it.
        let mut slower = 0;
        for _ in 0..64 {
            slower = p.frame(4 * MS, None, None);
        }
        assert!(slower > steady);
        assert!(
            p.ceiling() > Duration::from_micros(390),
            "{:?}",
            p.ceiling()
        );
    }

    #[test]
    fn a_stall_does_not_fill_the_sustained_cap() {
        let config = PacingConfig {
            sustained_bytes_per_s: 4 << 20,
            ..PacingConfig::DISCRETE
        };
        let mut p = Pacing::new(config);
        for _ in 0..16 {
            p.frame(2 * MS, None, None);
        }
        // Spend the whole burst: what comes back in a frame is a frame's worth.
        p.spent(1 << 20);
        let frame_worth = (4u64 << 20) / 500;
        let after = p.frame(2 * MS, None, None);
        assert!(after <= frame_worth + 1, "{after}");
        p.spent(after);
        // A two-second stall fills no more than a normal frame does.
        let after = p.frame(Duration::from_secs(2), None, None);
        assert!(after <= frame_worth + 1, "{after}");
    }

    #[test]
    fn less_time_before_the_deadline_admits_less() {
        let mut p = Pacing::new(PacingConfig::DISCRETE);
        let full = p.frame(4 * MS, Some(4 * MS), None);
        let late = p.frame(4 * MS, Some(4 * MS), Some(Duration::from_micros(100)));
        assert_eq!(p.ceiling(), Duration::from_micros(100));
        assert!(late < full);
        assert_eq!(p.frame(4 * MS, Some(4 * MS), Some(Duration::ZERO)), 0);
    }

    #[test]
    fn the_write_speed_is_measured_from_large_writes_only() {
        let mut p = Pacing::new(PacingConfig::DISCRETE);
        p.wrote(100, Duration::from_secs(1));
        assert_eq!(p.write_bytes_per_s(), 2e9);
        for _ in 0..64 {
            p.wrote(1 << 20, Duration::from_micros(1048));
        }
        let speed = p.write_bytes_per_s();
        assert!((0.99e9..1.01e9).contains(&speed), "{speed}");
    }

    #[test]
    fn the_phases_admit_differently() {
        let mut b = Budget::new(1000, PacingConfig::DISCRETE);
        assert_eq!(b.phase(), UploadPhase::Gameplay);
        // Gameplay without a reported pace: the fixed ceiling, per frame.
        assert!(b.admit(600, 1));
        b.spend(600);
        assert!(!b.admit(600, 1));
        assert_eq!(b.refused, 1);
        // A submission between frames does not renew it; a recorded frame does.
        b.submitted();
        assert!(!b.admit(600, 1));
        b.frame_recorded();
        assert!(b.admit(600, 2));
        b.spend(600);
        // Loading: bursts with no ceiling of their own.
        b.set_phase(UploadPhase::Loading);
        assert!(b.admit(1 << 30, 2));
        assert_eq!(b.remaining(), u64::MAX - 600);
        // Back in gameplay, a reported pace lowers the ceiling for its frame only.
        b.set_phase(UploadPhase::Gameplay);
        b.frame_recorded();
        b.pace(Duration::from_micros(480), None, Some(Duration::ZERO));
        assert_eq!(b.allowance(), 0);
        assert!(b.admit(600, 3), "the first upload of a frame always goes");
        b.spend(600);
        assert!(!b.admit(1, 3));
        b.frame_recorded();
        assert_eq!(b.allowance(), 1000);
    }
}
