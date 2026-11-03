//! Keep silent media decoders stopped, without cutting off an audible fade.
pub const FADE_SECONDS: f64 = 0.010;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Keep,
    Play,
    Pause,
}

#[derive(Default)]
pub struct Gain {
    target: f32,
    silent_after: f64,
}
impl Gain {
    /// Repeated UI ticks must not restart an in-flight gain envelope.
    pub fn update(&mut self, target: f32, now: f64) -> bool {
        if self.target == target {
            return false;
        }
        self.target = target;
        self.silent_after = now + FADE_SECONDS;
        true
    }
    pub fn activity(&self, playing: bool, paused: bool, now: f64) -> Activity {
        if !playing || (self.target == 0. && now >= self.silent_after) {
            if paused {
                Activity::Keep
            } else {
                Activity::Pause
            }
        } else if self.target > 0. && paused {
            Activity::Play
        } else {
            Activity::Keep
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_playback_leaves_all_silent_stem_decoders_stopped() {
        let gain = Gain::default();
        assert_eq!(gain.activity(true, true, 5.), Activity::Keep);
        assert_eq!(gain.activity(true, false, 5.), Activity::Pause);
    }

    #[test]
    fn muting_finishes_the_fade_before_pausing_and_ticks_cannot_extend_it() {
        let mut gain = Gain::default();
        gain.update(1., 0.);
        assert!(gain.update(0., 1.));
        assert_eq!(gain.activity(true, false, 1.005), Activity::Keep);
        assert!(!gain.update(0., 1.006));
        assert_eq!(gain.activity(true, false, 1.011), Activity::Pause);
    }

    #[test]
    fn unmute_during_a_fade_keeps_playing_and_paused_channels_rejoin() {
        let mut gain = Gain::default();
        gain.update(1., 0.);
        gain.update(0., 1.);
        gain.update(0.18, 1.005);
        assert_eq!(gain.activity(true, false, 1.02), Activity::Keep);
        assert_eq!(gain.activity(true, true, 1.02), Activity::Play);
        assert_eq!(gain.activity(false, false, 1.02), Activity::Pause);
        assert_eq!(gain.activity(false, true, 1.02), Activity::Keep);
    }
}
