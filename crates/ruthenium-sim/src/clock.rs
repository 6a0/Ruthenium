//! The tick clock (`DeltaTracker.Timer`): game ticks counted on vanilla's millisecond
//! clock, so they fall on the same milliseconds as a vanilla client's. Drawing
//! interpolates on its own nanosecond clock; this one only decides when ticks run.

/// Vanilla's 20 ticks a second.
pub const DEFAULT_MS_PER_TICK: f32 = 50.0;
/// The most ticks run at once after a stall (`Minecraft.runTick`); the rest are dropped.
pub const MAX_TICKS_AT_ONCE: u32 = 10;

#[derive(Debug, Clone)]
pub struct TickClock {
    last_ms: i64,
    residual: f32,
}

impl TickClock {
    pub fn new(now_ms: i64) -> TickClock {
        TickClock {
            last_ms: now_ms,
            residual: 0.0,
        }
    }

    /// Ticks due at `now_ms` at `ms_per_tick` (`advanceGameTime`), already capped at
    /// [`MAX_TICKS_AT_ONCE`] as `runTick` caps them.
    pub fn advance(&mut self, now_ms: i64, ms_per_tick: f32) -> u32 {
        // Java: (float)(currentMs - lastMs) / mspt, in floats.
        let delta = (now_ms - self.last_ms) as f32 / ms_per_tick;
        self.last_ms = now_ms;
        self.residual += delta;
        let ticks = self.residual as i32;
        self.residual -= ticks as f32;
        (ticks.max(0) as u32).min(MAX_TICKS_AT_ONCE)
    }

    /// The millisecond at which the next tick falls due.
    pub fn next_tick_ms(&self, ms_per_tick: f32) -> i64 {
        let need = ((1.0 - self.residual) * ms_per_tick).max(0.0);
        self.last_ms + need.ceil() as i64
    }
}

/// The tick length the game runs at (`Minecraft.getTickTargetMillis`): the server's
/// rate when it runs normally (never faster than vanilla's 20 a second), else 20 a second.
pub fn ms_per_tick(tick_rate: f32, runs_normally: bool) -> f32 {
    if !runs_normally {
        return DEFAULT_MS_PER_TICK;
    }
    // `TickRateManager`: nanoseconds per tick as a long, then milliseconds as a float.
    let rate = tick_rate.max(1.0);
    let nanos = (1_000_000_000f64 / f64::from(rate)) as i64;
    let ms = nanos as f32 / 1_000_000f32;
    DEFAULT_MS_PER_TICK.max(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_fall_on_the_millisecond_clock() {
        let mut c = TickClock::new(1000);
        assert_eq!(c.advance(1049, 50.0), 0);
        assert_eq!(c.next_tick_ms(50.0), 1050);
        assert_eq!(c.advance(1050, 50.0), 1);
        assert_eq!(c.advance(1175, 50.0), 2);
        // Half a tick is left over.
        assert_eq!(c.next_tick_ms(50.0), 1200);
        // A stall runs ten ticks and drops the rest.
        assert_eq!(c.advance(3000, 50.0), 10);
        assert_eq!(c.advance(3010, 50.0), 0);
    }

    #[test]
    fn tick_rates() {
        assert_eq!(ms_per_tick(20.0, true), 50.0);
        // Faster than 20 a second still ticks at 20 on the client.
        assert_eq!(ms_per_tick(40.0, true), 50.0);
        assert_eq!(ms_per_tick(10.0, true), 100.0);
        assert_eq!(ms_per_tick(3.0, true), 333.33334);
        assert_eq!(ms_per_tick(10.0, false), 50.0);
    }
}
