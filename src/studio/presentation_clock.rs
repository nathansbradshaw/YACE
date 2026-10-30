//! Smooth visual transport between coarse audio-position observations.
//!
//! The audio counter remains authoritative. This is a presentation estimate,
//! not a measurement of sound reaching the listener. Call `reset` on every
//! seek, loop wrap, play transition, and playback-rate change. Sample only while
//! playing; use the actual paused position while paused.

const MAX_EXTRAPOLATION: f64 = 0.100;
const STALE_AFTER: f64 = 0.250;
const DISCIPLINE_TIME: f64 = 0.250;
const MAX_SLEW_FRACTION: f64 = 0.10;
const PROGRESS_EPSILON: f64 = 1e-9;

#[derive(Clone, Debug, Default)]
pub struct PresentationClock {
    initialized: bool,
    last_now: f64,
    position: f64,
    raw: f64,
    observed_at: f64,
    stale: bool,
}

impl PresentationClock {
    /// Reanchor after a deliberate transport discontinuity. Positions are source
    /// seconds, and `now` is seconds from a monotonic clock with a fixed epoch.
    pub fn reset(&mut self, now: f64, position: f64) {
        let now = if now.is_finite() { now } else { 0.0 };
        let position = if position.is_finite() {
            position.max(0.0)
        } else {
            0.0
        };
        *self = Self {
            initialized: true,
            last_now: now,
            position,
            raw: position,
            observed_at: now,
            stale: false,
        };
    }

    /// Estimate a smooth source position from a fresh monotonic-clock sample.
    ///
    /// `raw` must already be expressed in source seconds (including any playback
    /// rate conversion). Forward audio observations update the reference; tiny
    /// regressions and repeated observations do not reset its age. The reference
    /// extrapolates for at most 100 ms of wall time, then holds. A large unexpected
    /// forward counter jump can reanchor within that same bound; backward jumps
    /// require an explicit `reset`.
    pub fn sample(&mut self, now: f64, raw: f64, rate: f64) -> f64 {
        if !now.is_finite() {
            return self.position;
        }
        if !self.initialized {
            self.reset(now, raw);
            return self.position;
        }
        if now < self.last_now {
            return self.position;
        }
        let elapsed = now - self.last_now;
        self.last_now = now;
        if raw.is_finite() && raw >= 0.0 && raw > self.raw + PROGRESS_EPSILON {
            self.raw = raw;
            self.observed_at = now;
        }
        let age = now - self.observed_at;
        self.stale = age >= STALE_AFTER;
        if !rate.is_finite() || rate <= 0.0 || self.stale {
            return self.position;
        }

        // Advance the last audio observation before correcting the playhead.
        // Reusing its old buffer position would pull the display backward.
        let reference = self.raw + age.min(MAX_EXTRAPOLATION) * rate;
        let predicted = self.position + elapsed * rate;
        let blend = -(-elapsed / DISCIPLINE_TIME).exp_m1();
        let slew_limit = elapsed * rate * MAX_SLEW_FRACTION;
        let correction = ((reference - predicted) * blend).clamp(-slew_limit, slew_limit);
        let lead_bound = self.raw + MAX_EXTRAPOLATION * rate;
        let lag_bound = (self.raw - MAX_EXTRAPOLATION * rate).max(0.0);
        self.position = (predicted + correction)
            .min(lead_bound)
            .max(lag_bound)
            .max(self.position);
        self.position
    }

    /// Whether the audio counter has failed to advance for at least 250 ms.
    /// This reflects the last `sample`, not an independently running timer.
    pub fn is_stale(&self) -> bool {
        self.stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn coarse_audio_batches_produce_smooth_monotonic_display_at_each_rate() {
        for rate in [0.5, 1.0, 1.5] {
            let mut clock = PresentationClock::default();
            clock.reset(0.0, 12.0);
            let mut previous = 12.0;
            for frame in 1..=1200 {
                let now = frame as f64 / 60.0;
                let raw = 12.0 + (now / 0.040).floor() * 0.040 * rate;
                let position = clock.sample(now, raw, rate);
                let advance = position - previous;
                assert!(
                    advance > 0.0,
                    "stalled display at frame {frame}, rate {rate}"
                );
                assert!((advance - rate / 60.0).abs() < rate * 0.002);
                assert!((position - (12.0 + now * rate)).abs() < rate * 0.040);
                assert!(position <= raw + MAX_EXTRAPOLATION * rate + 1e-9);
                assert!(!clock.is_stale());
                previous = position;
            }
        }
    }

    #[test]
    fn closely_spaced_timer_and_frame_samples_do_not_double_advance() {
        let mut single = PresentationClock::default();
        let mut split = PresentationClock::default();
        single.reset(0.0, 0.0);
        split.reset(0.0, 0.0);
        for frame in 1..=600 {
            let now = frame as f64 / 60.0;
            let raw = (now / 0.040).floor() * 0.040;
            let a = single.sample(now, raw, 1.0);
            let b = split.sample(now, raw, 1.0);
            near(split.sample(now, raw, 1.0), b);
            let c = split.sample(now + 0.0001, raw, 1.0);
            assert!(c >= b && c - b <= 0.00011);
            assert!((c - a).abs() < 0.002);
        }
    }

    #[test]
    fn frozen_audio_has_bounded_interpolation_then_holds_and_marks_stale() {
        for rate in [0.5, 1.0, 1.5] {
            let mut clock = PresentationClock::default();
            clock.reset(0.0, 5.0);
            let mut held = 0.0;
            for frame in 1..=120 {
                let now = frame as f64 / 60.0;
                let position = clock.sample(now, 5.0, rate);
                assert!(position <= 5.0 + MAX_EXTRAPOLATION * rate + 1e-9);
                if frame == 12 {
                    held = position;
                    assert!(!clock.is_stale());
                } else if frame > 12 {
                    near(position, held);
                }
                if frame >= 15 {
                    assert!(clock.is_stale());
                }
            }
            let resumed = clock.sample(2.01, 7.0, rate);
            assert!(!clock.is_stale());
            assert!(resumed >= 7.0 - MAX_EXTRAPOLATION * rate);
            assert!(resumed <= 7.0 + MAX_EXTRAPOLATION * rate);
        }
    }

    #[test]
    fn explicit_resets_allow_seek_loop_and_rate_discontinuities() {
        let mut clock = PresentationClock::default();
        clock.reset(1.0, 30.0);
        near(clock.sample(1.02, 30.0, 1.0), 30.02);
        clock.reset(1.02, 3.0); // Backward seek or loop wrap.
        near(clock.sample(1.02, 3.0, 1.0), 3.0);
        near(clock.sample(1.04, 3.0, 1.0), 3.02);
        clock.reset(10.0, 50.0); // Play transition after a pause.
        near(clock.sample(10.0, 50.0, 0.5), 50.0);
        near(clock.sample(10.02, 50.0, 0.5), 50.01);
        clock.reset(10.02, 50.01); // Explicit playback-rate change.
        near(clock.sample(10.04, 50.01, 1.5), 50.04);
        assert!(!clock.is_stale());
    }

    #[test]
    fn backward_counter_jitter_does_not_rewind_or_extend_freshness() {
        let mut clock = PresentationClock::default();
        clock.reset(0.0, 10.0);
        let a = clock.sample(0.04, 10.04, 1.0);
        let b = clock.sample(0.06, 10.039, 1.0);
        assert!(b >= a);
        let held = clock.sample(0.20, 10.039, 1.0);
        near(clock.sample(0.30, 10.039, 1.0), held);
        assert!(clock.is_stale());
        near(clock.sample(0.25, 20.0, 1.0), held); // Bad monotonic timestamp.
        assert!(clock.is_stale());
    }

    #[test]
    fn malformed_samples_hold_finite_position_without_catchup() {
        let mut clock = PresentationClock::default();
        near(clock.sample(1.0, 3.0, 1.0), 3.0);
        near(clock.sample(f64::NAN, 3.0, 1.0), 3.0);
        near(clock.sample(1.02, f64::NAN, 0.0), 3.0);
        let next = clock.sample(1.04, 3.0, 1.0);
        assert!((3.0..3.022).contains(&next));
        near(clock.sample(1.05, 3.0, f64::INFINITY), next);
    }
}
