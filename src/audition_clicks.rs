//! Local audition signals, independent of authored notes and playable exports.
//! Both clients use source seconds for placement and output frames for synthesis.
use crate::{sync_track::Meter, timing::TempoMap};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[path = "audition_clicks/mix.rs"]
mod mix;
pub use mix::Mix;
#[path = "audition_clicks/preferences.rs"]
pub mod preferences;
#[path = "audition_clicks/project.rs"]
mod project;

pub const MAX_EVENTS: usize = 100_000;
pub const CLICK_SECONDS: f64 = 0.024;
pub const PEAK: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteClass {
    Instrument,
    Open,
    Kick,
    DrumPad,
    PitchedVocal,
    Talkie,
    VocalPercussion,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Singer {
    #[default]
    Lead,
    Harm1,
    Harm2,
    Harm3,
}
impl NoteClass {
    pub const ALL: [Self; 7] = [
        Self::Instrument,
        Self::Open,
        Self::Kick,
        Self::DrumPad,
        Self::PitchedVocal,
        Self::Talkie,
        Self::VocalPercussion,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Instrument => "Instrument notes",
            Self::Open => "Open notes",
            Self::Kick => "Drum kicks",
            Self::DrumPad => "Drum pads",
            Self::PitchedVocal => "Pitched vocals",
            Self::Talkie => "Talkies",
            Self::VocalPercussion => "Vocal percussion",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub metronome: bool,
    pub claps: bool,
    pub beat_gain: f32,
    pub clap_gain: f32,
    pub classes: Vec<NoteClass>,
    pub singer: Singer,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            metronome: false,
            claps: false,
            beat_gain: 0.5,
            clap_gain: 0.5,
            classes: NoteClass::ALL.into(),
            singer: Singer::Lead,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [self.beat_gain, self.clap_gain]
                .into_iter()
                .all(|g| g.is_finite() && (0. ..=1.).contains(&g)),
            "Audition gains must be between 0 and 1"
        );
        ensure!(
            self.classes.len() <= NoteClass::ALL.len(),
            "Too many audition note classes"
        );
        for (i, class) in self.classes.iter().enumerate() {
            ensure!(
                !self.classes[..i].contains(class),
                "Duplicate audition note class"
            );
        }
        Ok(())
    }
    pub fn enabled(&self) -> bool {
        self.metronome && self.beat_gain > 0. || self.claps && self.clap_gain > 0.
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Beat,
    Accent,
    Clap,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub seconds: f64,
    pub kind: Kind,
}
#[derive(Clone, Debug)]
pub struct Plan {
    events: Vec<Event>,
    duration: f64,
}
struct Anchor {
    tick: f64,
    seconds: f64,
    step: f64,
}
struct Clock(Vec<Anchor>);
impl Clock {
    fn new(timing: &TempoMap) -> Result<Self> {
        ensure!(
            TempoMap::new(timing.resolution, timing.offset, timing.tempos.clone())? == *timing,
            "Invalid audition tempo map"
        );
        let mut seconds = timing.offset;
        let mut anchors: Vec<Anchor> = Vec::with_capacity(timing.tempos.len());
        for tempo in &timing.tempos {
            let tick = tempo.tick as f64;
            if let Some(last) = anchors.last() {
                seconds += (tick - last.tick) * last.step;
            }
            anchors.push(Anchor {
                tick,
                seconds,
                step: 60. / (tempo.bpm * timing.resolution as f64),
            });
        }
        Ok(Self(anchors))
    }
    fn seconds(&self, tick: f64) -> f64 {
        let a = &self.0[self.0.partition_point(|a| a.tick <= tick).saturating_sub(1)];
        a.seconds + (tick - a.tick) * a.step
    }
}
impl Plan {
    pub fn new(mut events: Vec<Event>, duration: f64) -> Result<Self> {
        ensure!(
            duration.is_finite() && duration > 0. && duration <= 86400.,
            "Invalid audition timeline length"
        );
        ensure!(
            events.len() <= MAX_EVENTS,
            "Audition schedule exceeds 100,000 events"
        );
        ensure!(
            events.iter().all(|e| e.seconds.is_finite()),
            "Invalid audition event time"
        );
        events.retain(|e| e.seconds >= 0. && e.seconds < duration);
        events.sort_by(|a, b| {
            a.seconds
                .total_cmp(&b.seconds)
                .then_with(|| (a.kind as u8).cmp(&(b.kind as u8)))
        });
        events.dedup_by(|a, b| a.seconds == b.seconds && a.kind == b.kind);
        Ok(Self { events, duration })
    }
    /// Meter changes start a fresh bar. Denominator beats may fall between chart
    /// ticks; rounding them to integers would introduce avoidable timing error.
    pub fn from_ticks(
        timing: &TempoMap,
        meters: &[Meter],
        notes: &[u64],
        duration: f64,
    ) -> Result<Self> {
        ensure!(
            duration.is_finite() && duration > 0. && duration <= 86400.,
            "Invalid audition timeline length"
        );
        ensure!(
            !meters.is_empty()
                && meters[0].tick == 0
                && meters.windows(2).all(|m| m[0].tick < m[1].tick)
                && meters.iter().all(|m| m.numerator > 0
                    && m.denominator > 0
                    && m.denominator.is_power_of_two()),
            "Invalid audition meter map"
        );
        ensure!(
            notes.len() <= MAX_EVENTS,
            "Audition schedule exceeds 100,000 events"
        );
        let clock = Clock::new(timing)?;
        let mut events = Vec::new();
        for (index, meter) in meters.iter().enumerate() {
            let step = timing.resolution as f64 * 4. / meter.denominator as f64;
            let end = meters.get(index + 1).map(|m| m.tick as f64);
            let mut beat = 0u64;
            loop {
                let tick = meter.tick as f64 + beat as f64 * step;
                if end.is_some_and(|end| tick >= end) {
                    break;
                }
                let seconds = clock.seconds(tick);
                if seconds >= duration {
                    break;
                }
                if seconds >= 0. {
                    ensure!(
                        events.len() < MAX_EVENTS,
                        "Audition schedule exceeds 100,000 events"
                    );
                    events.push(Event {
                        seconds,
                        kind: if beat.is_multiple_of(u64::from(meter.numerator)) {
                            Kind::Accent
                        } else {
                            Kind::Beat
                        },
                    });
                }
                // A huge negative chart offset must not force billions of skipped
                // beats before reaching the playable timeline.
                if seconds < 0.
                    && let Some(first) = timing.tick_position(0.)
                {
                    let first = ((first - meter.tick as f64) / step).ceil().max(0.) as u64;
                    if first > beat {
                        beat = first;
                        continue;
                    }
                }
                beat = beat
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("Audition beat overflow"))?;
            }
        }
        let mut ticks = notes.to_vec();
        ticks.sort_unstable();
        ticks.dedup();
        for tick in ticks {
            let seconds = clock.seconds(tick as f64);
            if seconds >= 0. && seconds < duration {
                ensure!(
                    events.len() < MAX_EVENTS,
                    "Audition schedule exceeds 100,000 events"
                );
                events.push(Event {
                    seconds,
                    kind: Kind::Clap,
                });
            }
        }
        Self::new(events, duration)
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
    /// Render an output-rate mono block. `start` is a source-clock position;
    /// practice speed changes placement without changing the click's pitch.
    /// Blocks can be rendered independently, including after seek or loop wrap.
    pub fn render(
        &self,
        start: f64,
        rate: f64,
        sample_rate: u32,
        settings: &Settings,
        output: &mut [f32],
    ) -> Result<()> {
        settings.validate()?;
        ensure!(
            start.is_finite()
                && rate.is_finite()
                && (0.25..=2.).contains(&rate)
                && (8000..=192000).contains(&sample_rate),
            "Invalid audition render clock"
        );
        ensure!(
            output.len() <= sample_rate as usize * 2,
            "Audition blocks must be at most two seconds"
        );
        output.fill(0.);
        if !settings.enabled() || output.is_empty() {
            return Ok(());
        }
        let end = start + output.len() as f64 * rate / sample_rate as f64;
        let first = self
            .events
            .partition_point(|e| e.seconds < start - CLICK_SECONDS * rate);
        let last = self.events.partition_point(|e| e.seconds < end);
        let tail = (CLICK_SECONDS * sample_rate as f64).ceil() as i64;
        for event in &self.events[first..last] {
            let gain = match event.kind {
                Kind::Clap if settings.claps => settings.clap_gain,
                Kind::Beat | Kind::Accent if settings.metronome => settings.beat_gain,
                _ => continue,
            };
            let onset = ((event.seconds - start) / rate * sample_rate as f64).round() as i64;
            for age in (-onset).max(0)..tail.min(output.len() as i64 - onset) {
                let index = (onset + age) as usize;
                let source_seconds = start + index as f64 * rate / sample_rate as f64;
                if source_seconds >= 0. && source_seconds < self.duration {
                    output[index] += gain * pulse(event.kind, age as u32, sample_rate);
                }
            }
        }
        for value in output {
            *value = value.clamp(-PEAK, PEAK);
        }
        Ok(())
    }
}
pub fn pulse(kind: Kind, frame: u32, sample_rate: u32) -> f32 {
    let t = frame as f64 / sample_rate as f64;
    if !(0. ..CLICK_SECONDS).contains(&t) {
        return 0.;
    }
    let envelope = (t / 0.001).min(1.) * (-t / 0.006).exp();
    let signal = match kind {
        Kind::Beat => (std::f64::consts::TAU * 1100. * t).sin(),
        Kind::Accent => (std::f64::consts::TAU * 1700. * t).sin(),
        Kind::Clap => {
            let mut x = frame.wrapping_add(0x9e37_79b9);
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            0.7 * (f64::from(x) / f64::from(u32::MAX) * 2. - 1.)
                + 0.3 * (std::f64::consts::TAU * 2300. * t).sin()
        }
    };
    (0.12 * envelope * signal) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::Tempo;
    fn meters() -> Vec<Meter> {
        vec![Meter {
            tick: 0,
            numerator: 4,
            denominator: 4,
        }]
    }
    #[test]
    fn tempo_meter_changes_and_chords_keep_the_source_clock() {
        let timing = TempoMap::new(
            480,
            -0.25,
            vec![
                Tempo { tick: 0, bpm: 120. },
                Tempo {
                    tick: 960,
                    bpm: 60.,
                },
            ],
        )
        .unwrap();
        let mut m = meters();
        m.push(Meter {
            tick: 1440,
            numerator: 3,
            denominator: 8,
        });
        let plan = Plan::from_ticks(&timing, &m, &[480, 480, 960], 4.).unwrap();
        let beats: Vec<_> = plan
            .events()
            .iter()
            .filter(|e| e.kind != Kind::Clap)
            .map(|e| (e.seconds, e.kind))
            .collect();
        assert_eq!(
            beats,
            vec![
                (0.25, Kind::Beat),
                (0.75, Kind::Beat),
                (1.75, Kind::Accent),
                (2.25, Kind::Beat),
                (2.75, Kind::Beat),
                (3.25, Kind::Accent),
                (3.75, Kind::Beat)
            ]
        );
        assert_eq!(
            plan.events()
                .iter()
                .filter(|e| e.kind == Kind::Clap)
                .map(|e| e.seconds)
                .collect::<Vec<_>>(),
            vec![0.25, 0.75]
        );
        assert_eq!(timing.offset, -0.25);
    }
    #[test]
    fn fractional_meter_beats_are_not_rounded_to_chart_ticks() {
        let timing = TempoMap::new(100, 0., vec![]).unwrap();
        let plan = Plan::from_ticks(
            &timing,
            &[Meter {
                tick: 0,
                numerator: 3,
                denominator: 128,
            }],
            &[],
            1.,
        )
        .unwrap();
        assert_eq!(plan.events()[1].seconds, 0.015625);
        assert_eq!(plan.events()[3].kind, Kind::Accent);
    }
    #[test]
    fn short_recordings_and_large_negative_offsets_remain_bounded() {
        let short =
            Plan::from_ticks(&TempoMap::constant(120.).unwrap(), &meters(), &[], 0.25).unwrap();
        assert_eq!(
            short.events(),
            &[Event {
                seconds: 0.,
                kind: Kind::Accent
            }]
        );
        let timing = TempoMap::new(480, -1_000_000., vec![]).unwrap();
        let plan = Plan::from_ticks(&timing, &meters(), &[], 1.).unwrap();
        assert_eq!(plan.events().len(), 2);
        assert_eq!(plan.events()[0].seconds, 0.);
        assert_eq!(plan.events()[1].seconds, 0.5);
    }
    #[test]
    fn practice_rate_moves_attacks_without_transposing_the_pulse() {
        let plan = Plan::new(
            vec![Event {
                seconds: 1.,
                kind: Kind::Clap,
            }],
            3.,
        )
        .unwrap();
        let s = Settings {
            claps: true,
            clap_gain: 1.,
            ..Settings::default()
        };
        let mut normal = vec![0.; 48000];
        let mut fast = normal.clone();
        plan.render(0.5, 1., 48000, &s, &mut normal).unwrap();
        plan.render(0.5, 2., 48000, &s, &mut fast).unwrap();
        assert!(normal[..24000].iter().all(|&x| x == 0.));
        assert!(fast[..12000].iter().all(|&x| x == 0.));
        assert_eq!(&normal[24000..25152], &fast[12000..13152]);
    }
    #[test]
    fn independent_blocks_and_loop_restarts_retain_the_same_samples() {
        let plan = Plan::new(
            vec![Event {
                seconds: 0.49,
                kind: Kind::Clap,
            }],
            2.,
        )
        .unwrap();
        let s = Settings {
            claps: true,
            ..Settings::default()
        };
        let mut whole = vec![0.; 48000];
        let mut a = vec![0.; 24000];
        let mut b = a.clone();
        plan.render(0., 1., 48000, &s, &mut whole).unwrap();
        plan.render(0., 1., 48000, &s, &mut a).unwrap();
        plan.render(0.5, 1., 48000, &s, &mut b).unwrap();
        a.extend_from_slice(&b);
        assert_eq!(whole, a);
        let mut again = vec![0.; 48000];
        plan.render(0., 1., 48000, &s, &mut again).unwrap();
        assert_eq!(whole, again);
    }
    #[test]
    fn mute_gains_dense_headroom_and_refusals_are_explicit() {
        let plan = Plan::new(
            (0..1000)
                .map(|i| Event {
                    seconds: 0.5 + i as f64 * 0.000001,
                    kind: Kind::Clap,
                })
                .collect(),
            2.,
        )
        .unwrap();
        let mut out = vec![1.; 48000];
        plan.render(0., 1., 48000, &Settings::default(), &mut out)
            .unwrap();
        assert!(out.iter().all(|&x| x == 0.));
        plan.render(
            0.,
            1.,
            48000,
            &Settings {
                claps: true,
                clap_gain: 1.,
                ..Settings::default()
            },
            &mut out,
        )
        .unwrap();
        assert!(out.iter().any(|&x| x != 0.));
        assert!(out.iter().all(|x| x.abs() <= PEAK));
        assert!(
            plan.render(f64::NAN, 1., 48000, &Settings::default(), &mut out)
                .is_err()
        );
        assert!(
            Settings {
                beat_gain: f32::NAN,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Plan::new(
                vec![Event {
                    seconds: f64::INFINITY,
                    kind: Kind::Beat
                }],
                2.
            )
            .is_err()
        );
        assert!(
            Plan::from_ticks(&TempoMap::constant(1000.).unwrap(), &meters(), &[], 86400.).is_err()
        );
    }
}
