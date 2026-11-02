//! Follow a media clock without repeatedly interrupting the decoder with seeks.
//!
//! Media-element timestamps can be quantized and seeks finish asynchronously.
//! Small, persistent drift is corrected through pitch-preserving playback rate;
//! only a sustained large offset warrants a discontinuous seek.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction {
    None,
    Rate(f64),
    Seek(f64),
}

#[derive(Default)]
pub struct Follower {
    drift_since: Option<(f64, bool)>,
    last_seek: Option<f64>,
    last_rate_change: Option<f64>,
    factor: Option<f64>,
}

impl Follower {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(
        &mut self,
        now: f64,
        leader: f64,
        follower: f64,
        base_rate: f64,
        running: bool,
    ) -> Correction {
        if !running
            || ![now, leader, follower, base_rate]
                .iter()
                .all(|v| v.is_finite())
            || base_rate <= 0.
        {
            self.drift_since = None;
            return self.rate(now, base_rate, 1.);
        }
        let drift = leader - follower;
        if drift.abs() <= 0.03 {
            self.drift_since = None;
            return self.rate(now, base_rate, 1.);
        }
        let ahead = drift > 0.;
        let since = match self.drift_since {
            Some((since, sign)) if sign == ahead && now >= since => since,
            _ => {
                self.drift_since = Some((now, ahead));
                return Correction::None;
            }
        };
        // Ignore startup skew and isolated/rounded currentTime observations.
        if now - since < 0.4 {
            return Correction::None;
        }
        if drift.abs() > 0.25 && self.last_seek.is_none_or(|last| now - last >= 2.) {
            self.last_seek = Some(now);
            self.drift_since = None;
            self.factor = Some(1.);
            self.last_rate_change = Some(now);
            return Correction::Seek(leader);
        }
        self.rate(now, base_rate, 1. + (drift * 0.5).clamp(-0.02, 0.02))
    }

    fn rate(&mut self, now: f64, base: f64, factor: f64) -> Correction {
        if !now.is_finite() || !base.is_finite() || base <= 0. {
            return Correction::None;
        }
        let previous = self.factor.unwrap_or(1.);
        if (previous - factor).abs() < 0.001
            || self.last_rate_change.is_some_and(|last| now - last < 0.25)
        {
            return Correction::None;
        }
        self.factor = Some(factor);
        self.last_rate_change = Some(now);
        Correction::Rate(base * factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantized_clock_skew_does_not_restart_audio() {
        let mut clock = Follower::default();
        for tick in 0..1000 {
            let now = tick as f64 * 0.016;
            // Different media clocks cross their 100 ms timestamp boundaries
            // on different ticks, despite continuously rendering audio.
            let leader = (now * 10.).floor() / 10.;
            let follower = ((now - 0.02).max(0.) * 10.).floor() / 10.;
            assert_eq!(
                clock.update(now, leader, follower, 1., true),
                Correction::None
            );
        }
    }

    #[test]
    fn small_persistent_drift_is_slewed_without_a_seek() {
        for rate in [0.5, 1., 1.5] {
            let mut clock = Follower::default();
            let mut actual_rate = rate;
            let mut follower = 0.;
            let mut changed = false;
            for tick in 0..3000 {
                let now = tick as f64 * 0.016;
                let leader = now * rate + 0.08;
                match clock.update(now, leader, follower, rate, true) {
                    Correction::None => {}
                    Correction::Rate(value) => {
                        assert!((value / rate - 1.).abs() <= 0.020001);
                        actual_rate = value;
                        changed = true;
                    }
                    Correction::Seek(_) => panic!("Normal drift must not interrupt audio"),
                }
                follower += actual_rate * 0.016;
            }
            assert!(changed);
            assert!((follower - 48. * rate - 0.08).abs() < 0.031);
        }
    }

    #[test]
    fn delayed_seek_completion_cannot_cause_a_seek_storm() {
        let mut clock = Follower::default();
        let mut seek_until = 0.;
        let mut seeks = 0;
        for tick in 0..500 {
            let now = tick as f64 * 0.016;
            match clock.update(now, now + 1., now, 1., now >= seek_until) {
                Correction::Seek(_) => {
                    seeks += 1;
                    seek_until = now + 0.2;
                }
                Correction::Rate(_) | Correction::None => {}
            }
        }
        assert!((1..=4).contains(&seeks));
    }

    #[test]
    fn paused_buffering_and_seeking_channels_do_not_reseek() {
        let mut clock = Follower::default();
        for tick in 0..1000 {
            let now = tick as f64 * 0.016;
            assert_eq!(
                clock.update(now, now + 2., now, 1., false),
                Correction::None
            );
        }
        clock.reset();
        assert_eq!(clock.update(20., 40., 20., 1., true), Correction::None);
    }
}
