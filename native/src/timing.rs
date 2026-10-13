use crate::TICK_HZ;
use std::time::Duration;

/// Integer accumulator avoids rounding 1/60 second to an integral nanosecond.
#[derive(Default)]
pub struct FixedClock {
    scaled_ns: u128,
}

impl FixedClock {
    pub fn advance(&mut self, elapsed: Duration) -> usize {
        // Treat long pauses/debugger stops as a discontinuity; avoid an
        // unbounded catch-up loop. Normal cadence is independent of rendering.
        self.scaled_ns += elapsed.min(Duration::from_millis(100)).as_nanos() * u128::from(TICK_HZ);
        let ticks = self.scaled_ns / 1_000_000_000;
        self.scaled_ns %= 1_000_000_000;
        ticks as usize
    }

    /// One tick per display refresh near 60 Hz, with bounded catch-up for
    /// missed refreshes. Carry only stall time so fractional refresh rates
    /// keep their even cadence while repeated stalls cannot drain the audio.
    pub fn advance_display(&mut self, elapsed: Duration) -> usize {
        let nominal = Duration::from_secs_f64(1.0 / f64::from(TICK_HZ));
        if elapsed > nominal + nominal / 2 {
            1 + self.advance(elapsed.saturating_sub(nominal))
        } else {
            1
        }
    }

    /// Time counted since the last tick.
    pub fn since_tick(&self) -> Duration {
        Duration::from_nanos((self.scaled_ns / u128::from(TICK_HZ)) as u64)
    }

    /// Time until `advance` returns the next tick.
    pub fn until_tick(&self) -> Duration {
        Duration::from_nanos((1_000_000_000 - self.scaled_ns).div_ceil(u128::from(TICK_HZ)) as u64)
    }
}

/// Measure actual presentation intervals instead of SDL's rounded refresh rate.
/// A median tolerates isolated compositor stalls; present's blocking time also
/// distinguishes working vsync from an immediately-returning renderer.
#[derive(Default)]
pub struct RefreshProbe {
    periods: Vec<Duration>,
    waits: Vec<Duration>,
}

impl RefreshProbe {
    pub fn observe(&mut self, period: Duration, wait: Duration) -> Option<bool> {
        self.periods.push(period);
        self.waits.push(wait);
        if self.periods.len() < 120 {
            return None;
        }
        self.periods.sort_unstable();
        self.waits.sort_unstable();
        let period = self.periods[60].as_secs_f64();
        let nominal = 1.0 / f64::from(TICK_HZ);
        let near_sixty = (period / nominal - 1.0).abs() <= 0.005;
        let blocking = self.waits[60] >= Duration::from_micros(250);
        self.periods.clear();
        self.waits.clear();
        Some(near_sixty && blocking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_cadence_does_not_change_tick_count() {
        let mut smooth = FixedClock::default();
        let mut uneven = FixedClock::default();
        let a: usize = (0..1000)
            .map(|_| smooth.advance(Duration::from_millis(1)))
            .sum();
        let b: usize = (0..10)
            .flat_map(|_| [7, 13, 29, 51])
            .map(|ms| uneven.advance(Duration::from_millis(ms)))
            .sum();
        assert_eq!(a, 60);
        assert_eq!(b, a);
    }

    #[test]
    fn long_pause_has_bounded_catch_up() {
        assert_eq!(FixedClock::default().advance(Duration::from_secs(60)), 6);
    }

    #[test]
    fn reports_the_time_around_a_tick() {
        let mut clock = FixedClock::default();
        assert_eq!(clock.until_tick(), Duration::from_nanos(16_666_667));
        assert_eq!(clock.advance(Duration::from_millis(5)), 0);
        assert_eq!(clock.since_tick(), Duration::from_millis(5));
        assert_eq!(clock.until_tick(), Duration::from_nanos(11_666_667));
        // Waiting the reported time reaches the tick.
        assert_eq!(clock.advance(clock.until_tick()), 1);
        assert!(clock.since_tick() < Duration::from_micros(1));
        assert_eq!(clock.advance(Duration::from_millis(20)), 1);
        assert_eq!(clock.since_tick(), Duration::from_nanos(3_333_333));
    }

    #[test]
    fn refresh_probe_accepts_fractional_sixty_and_ignores_single_stall() {
        let mut probe = RefreshProbe::default();
        for frame in 0..120 {
            let period = if frame == 30 {
                Duration::from_millis(90)
            } else {
                Duration::from_secs_f64(1.0 / 59.95)
            };
            assert_eq!(
                probe.observe(period, period),
                (frame == 119).then_some(true)
            );
        }
    }

    #[test]
    fn refresh_probe_rejects_other_refresh_rates_and_broken_vsync() {
        for hz in [50.0, 59.0, 61.0, 120.0, 144.0, 1000.0] {
            let mut probe = RefreshProbe::default();
            let period = Duration::from_secs_f64(1.0 / hz);
            for _ in 0..119 {
                assert_eq!(probe.observe(period, period), None);
            }
            assert_eq!(probe.observe(period, period), Some(false));
        }
        let mut probe = RefreshProbe::default();
        for _ in 0..119 {
            probe.observe(Duration::from_secs_f64(1.0 / 60.0), Duration::ZERO);
        }
        assert_eq!(
            probe.observe(Duration::from_secs_f64(1.0 / 60.0), Duration::ZERO),
            Some(false)
        );
    }
}
