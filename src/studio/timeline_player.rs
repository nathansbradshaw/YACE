//! Keep chart-only transport independent of audio files and output devices.
use crate::timeline_clock::TimelineClock;
use anyhow::Result;
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

pub(super) struct Player(Mode);
enum Mode {
    Recording(rodio::Player),
    Timeline {
        clock: RefCell<TimelineClock>,
        epoch: Instant,
    },
}
impl Player {
    pub(super) fn recorded(player: rodio::Player) -> Self {
        Self(Mode::Recording(player))
    }
    pub(super) fn timeline(duration: f64, rate: f64) -> Result<Self> {
        let mut clock = TimelineClock::new(duration, 0.)?;
        clock.set_rate(rate, 0.)?;
        Ok(Self(Mode::Timeline {
            clock: RefCell::new(clock),
            epoch: Instant::now(),
        }))
    }
    pub(super) fn play(&self) {
        match &self.0 {
            Mode::Recording(player) => player.play(),
            Mode::Timeline { clock, epoch } => {
                let _ = clock
                    .borrow_mut()
                    .set_playing(true, epoch.elapsed().as_secs_f64());
            }
        }
    }
    pub(super) fn pause(&self) {
        match &self.0 {
            Mode::Recording(player) => player.pause(),
            Mode::Timeline { clock, epoch } => {
                let _ = clock
                    .borrow_mut()
                    .set_playing(false, epoch.elapsed().as_secs_f64());
            }
        }
    }
    pub(super) fn get_pos(&self) -> Duration {
        match &self.0 {
            Mode::Recording(player) => player.get_pos(),
            Mode::Timeline { clock, epoch } => {
                let clock = clock.borrow();
                Duration::from_secs_f64(
                    clock.raw_position(epoch.elapsed().as_secs_f64()) / clock.rate(),
                )
            }
        }
    }
    pub(super) fn empty(&self) -> bool {
        match &self.0 {
            Mode::Recording(player) => player.empty(),
            Mode::Timeline { clock, epoch } => clock.borrow().ended(epoch.elapsed().as_secs_f64()),
        }
    }
    pub(super) fn is_paused(&self) -> bool {
        match &self.0 {
            Mode::Recording(player) => player.is_paused(),
            Mode::Timeline { clock, .. } => clock.borrow().paused(),
        }
    }
    pub(super) fn set_volume(&self, volume: f32) {
        if let Mode::Recording(player) = &self.0 {
            player.set_volume(volume);
        }
    }
    pub(super) fn append(&self, source: impl rodio::Source<Item = f32> + Send + 'static) {
        if let Mode::Recording(player) = &self.0 {
            player.append(source);
        }
    }
    pub(super) fn try_seek(&self, position: Duration) -> Result<()> {
        match &self.0 {
            Mode::Recording(player) => {
                player.try_seek(position)?;
            }
            Mode::Timeline { clock, epoch } => {
                let mut clock = clock.borrow_mut();
                let rate = clock.rate();
                clock.seek(position.as_secs_f64() * rate, epoch.elapsed().as_secs_f64())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chart_only_player_seeks_and_pauses_without_opening_audio_hardware() {
        let player = Player::timeline(60., 0.5).unwrap();
        player.try_seek(Duration::from_secs(4)).unwrap();
        assert_eq!(player.get_pos(), Duration::from_secs(4));
        assert!(player.is_paused());
        player.play();
        assert!(!player.is_paused());
        player.pause();
        assert!(player.is_paused());
        assert!(player.get_pos() >= Duration::from_secs(4));
    }
}
