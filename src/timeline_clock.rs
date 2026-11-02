//! A bounded editing timeline when no recording supplies a playback clock.
//! The host supplies monotonic seconds; this never creates or decodes audio.
use std::fmt;

pub const MAX_DURATION: f64 = 24. * 60. * 60.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    Duration,
    Time,
    Position,
    Rate,
}
impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Duration => "Timeline duration must be positive and no longer than 24 hours",
            Self::Time => "Timeline clock requires finite monotonic seconds",
            Self::Position => "Position is outside the chart timeline",
            Self::Rate => "Practice rate must be between 0.5 and 1.5",
        })
    }
}
impl std::error::Error for ClockError {}

#[derive(Clone, Debug)]
pub struct TimelineClock {
    duration: f64,
    anchor_time: f64,
    anchor_position: f64,
    rate: f64,
    playing: bool,
}
impl TimelineClock {
    pub fn new(duration: f64, now: f64) -> Result<Self, ClockError> {
        if !duration.is_finite() || !(0.0..=MAX_DURATION).contains(&duration) || duration == 0. {
            return Err(ClockError::Duration);
        }
        Self::time(now)?;
        Ok(Self {
            duration,
            anchor_time: now,
            anchor_position: 0.,
            rate: 1.,
            playing: false,
        })
    }
    fn time(now: f64) -> Result<(), ClockError> {
        if now.is_finite() && now >= 0. {
            Ok(())
        } else {
            Err(ClockError::Time)
        }
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn paused(&self) -> bool {
        !self.playing
    }
    /// Unclamped time lets the transport retain overshoot when wrapping a loop.
    pub fn raw_position(&self, now: f64) -> f64 {
        let elapsed = if self.playing && Self::time(now).is_ok() {
            (now - self.anchor_time).max(0.) * self.rate
        } else {
            0.
        };
        (self.anchor_position + elapsed).min(f64::MAX)
    }
    pub fn position(&self, now: f64) -> f64 {
        self.raw_position(now).min(self.duration)
    }
    pub fn ended(&self, now: f64) -> bool {
        self.playing && self.raw_position(now) >= self.duration
    }
    pub fn seek(&mut self, position: f64, now: f64) -> Result<(), ClockError> {
        Self::time(now)?;
        if !position.is_finite() || !(0.0..=self.duration).contains(&position) {
            return Err(ClockError::Position);
        }
        self.anchor_position = position;
        self.anchor_time = now;
        Ok(())
    }
    pub fn set_playing(&mut self, playing: bool, now: f64) -> Result<(), ClockError> {
        Self::time(now)?;
        self.seek(self.position(now), now)?;
        self.playing = playing;
        Ok(())
    }
    pub fn set_rate(&mut self, rate: f64, now: f64) -> Result<(), ClockError> {
        Self::time(now)?;
        if !rate.is_finite() || !(0.5..=1.5).contains(&rate) {
            return Err(ClockError::Rate);
        }
        self.seek(self.position(now), now)?;
        self.rate = rate;
        Ok(())
    }
    pub fn set_duration(&mut self, duration: f64, now: f64) -> Result<(), ClockError> {
        Self::new(duration, now)?;
        let position = self.position(now).min(duration);
        self.duration = duration;
        self.seek(position, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pause_rate_seek_and_resume_use_chart_seconds_without_audio() {
        let mut clock = TimelineClock::new(120., 10.).unwrap();
        assert_eq!(clock.position(30.), 0.);
        clock.set_playing(true, 30.).unwrap();
        assert_eq!(clock.position(34.), 4.);
        clock.set_rate(0.5, 34.).unwrap();
        assert_eq!(clock.position(38.), 6.);
        clock.set_playing(false, 38.).unwrap();
        assert_eq!(clock.position(99.), 6.);
        clock.seek(40., 100.).unwrap();
        clock.set_playing(true, 100.).unwrap();
        assert_eq!(clock.position(102.), 41.);
        clock.set_rate(1.5, 102.).unwrap();
        assert_eq!(clock.position(104.), 44.);
    }
    #[test]
    fn loop_overshoot_is_available_while_display_and_end_stay_bounded() {
        let mut clock = TimelineClock::new(10., 0.).unwrap();
        clock.seek(8., 0.).unwrap();
        clock.set_playing(true, 0.).unwrap();
        assert_eq!(clock.position(5.), 10.);
        assert_eq!(clock.raw_position(5.), 13.);
        assert!(clock.ended(5.));
        let wrapped = 2. + (clock.raw_position(5.) - 6.).rem_euclid(4.);
        clock.seek(wrapped, 5.).unwrap();
        assert_eq!(clock.position(5.), 5.);
        assert!(!clock.ended(5.));
    }
    #[test]
    fn invalid_changes_leave_clock_intact_and_duration_changes_are_bounded() {
        let mut clock = TimelineClock::new(MAX_DURATION, 0.).unwrap();
        clock.seek(100., 0.).unwrap();
        assert_eq!(clock.set_rate(f64::NAN, 0.), Err(ClockError::Rate));
        assert_eq!(clock.seek(-1., 0.), Err(ClockError::Position));
        assert_eq!(
            clock.set_playing(true, f64::INFINITY),
            Err(ClockError::Time)
        );
        assert_eq!(clock.set_duration(0., 0.), Err(ClockError::Duration));
        assert_eq!(clock.position(1.), 100.);
        assert!(clock.paused());
        assert_eq!(clock.rate(), 1.);
        clock.set_duration(20., 1.).unwrap();
        assert_eq!(clock.position(2.), 20.);
        clock.set_duration(MAX_DURATION, 3.).unwrap();
        assert_eq!(clock.position(4.), 20.);
        clock.set_playing(true, 4.).unwrap();
        assert!(clock.position(f64::MAX).is_finite());
    }
}
