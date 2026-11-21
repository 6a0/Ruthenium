use std::time::{Duration, Instant};

/// Shortest and longest time left to spin after the OS sleep.
const MIN_SPIN: Duration = Duration::from_micros(150);
const MAX_SPIN: Duration = Duration::from_millis(2);
const HISTORY: usize = 64;

/// Sleeps until a deadline with sub-millisecond accuracy: an OS sleep that stops short of
/// the deadline by a margin, then a spin for the rest. The margin adapts to how late the OS
/// has woken us recently (the worst of the last 64 sleeps), within 150 µs to 2 ms.
///
/// On Linux the calling thread's timer slack is set to 1 ns (default 50 µs); on Windows,
/// std's sleep already uses a high-resolution waitable timer.
#[derive(Debug)]
pub struct Sleeper {
    oversleep: [Duration; HISTORY],
    next: usize,
}

impl Default for Sleeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Sleeper {
    /// Creates a sleeper for the calling thread (and sets that thread's timer slack).
    pub fn new() -> Sleeper {
        #[cfg(target_os = "linux")]
        // SAFETY: PR_SET_TIMERSLACK only changes the calling thread's timer slack.
        unsafe {
            libc::prctl(libc::PR_SET_TIMERSLACK, 1 as libc::c_ulong);
        }
        Sleeper {
            oversleep: [MIN_SPIN; HISTORY],
            next: 0,
        }
    }

    /// The current spin margin.
    pub fn spin_margin(&self) -> Duration {
        let worst = self.oversleep.iter().copied().max().unwrap_or(MIN_SPIN);
        worst.clamp(MIN_SPIN, MAX_SPIN)
    }

    /// Returns at `deadline` (or immediately if it has passed).
    pub fn sleep_until(&mut self, deadline: Instant) {
        let margin = self.spin_margin();
        let now = Instant::now();
        if let Some(wake) = deadline.checked_sub(margin)
            && wake > now
        {
            std::thread::sleep(wake - now);
            self.oversleep[self.next] = Instant::now().saturating_duration_since(wake);
            self.next = (self.next + 1) % HISTORY;
        }
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
    }
}

/// The frame limiter's next deadline. Deadlines accumulate (`prev + period`) so the average
/// rate is exact and a late frame is paid back by the next ones; after falling more than one
/// whole period behind, the schedule restarts from `now` instead of bursting to catch up.
pub fn next_deadline(prev: Instant, period: Duration, now: Instant) -> Instant {
    let next = prev + period;
    if next + period < now { now } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_accumulate() {
        let t0 = Instant::now();
        let p = Duration::from_millis(4);
        // A frame that finished late (but within one period) keeps the original schedule.
        let d1 = next_deadline(t0, p, t0 + Duration::from_millis(6));
        assert_eq!(d1, t0 + p);
        let d2 = next_deadline(d1, p, t0 + Duration::from_millis(7));
        assert_eq!(d2, t0 + 2 * p);
    }

    #[test]
    fn deadlines_resync_after_a_real_miss() {
        let t0 = Instant::now();
        let p = Duration::from_millis(4);
        let now = t0 + Duration::from_millis(50);
        assert_eq!(next_deadline(t0, p, now), now);
    }

    #[test]
    fn sleeps_until_the_deadline() {
        let mut s = Sleeper::new();
        for _ in 0..5 {
            let deadline = Instant::now() + Duration::from_millis(3);
            s.sleep_until(deadline);
            assert!(Instant::now() >= deadline);
        }
        assert!(s.spin_margin() >= MIN_SPIN && s.spin_margin() <= MAX_SPIN);
    }
}
