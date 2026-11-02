//! Beat/audio/display clocks. Stop boundaries follow StepMania; cache layout
//! adapts SMEditor's before/of/after model (MIT, see THIRD_PARTY.md).
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimingKind {
    Bpms,
    Stops,
    Delays,
    Warps,
    Fakes,
    Speeds,
    Scrolls,
    Timesignatures,
    Labels,
    Tickcounts,
    Combos,
}
impl TimingKind {
    pub const ALL: [Self; 11] = [
        Self::Bpms,
        Self::Stops,
        Self::Delays,
        Self::Warps,
        Self::Fakes,
        Self::Speeds,
        Self::Scrolls,
        Self::Timesignatures,
        Self::Labels,
        Self::Tickcounts,
        Self::Combos,
    ];
    pub fn tag(self) -> &'static str {
        match self {
            Self::Bpms => "BPMS",
            Self::Stops => "STOPS",
            Self::Delays => "DELAYS",
            Self::Warps => "WARPS",
            Self::Fakes => "FAKES",
            Self::Speeds => "SPEEDS",
            Self::Scrolls => "SCROLLS",
            Self::Timesignatures => "TIMESIGNATURES",
            Self::Labels => "LABELS",
            Self::Tickcounts => "TICKCOUNTS",
            Self::Combos => "COMBOS",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.tag() == s)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub kind: TimingKind,
    pub beat: f64,
    pub values: Vec<f64>,
    #[serde(default)]
    pub text: String,
}
impl Event {
    pub fn value(kind: TimingKind, beat: f64, value: f64) -> Self {
        Self {
            kind,
            beat,
            values: vec![value],
            text: String::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.beat.is_finite() && self.beat.abs() <= crate::MAX_ROW as f64 / 48.,
            "Timing beat outside supported range"
        );
        ensure!(
            self.values.iter().all(|v| v.is_finite()),
            "Timing value must be finite"
        );
        let n = match self.kind {
            TimingKind::Labels => 0,
            TimingKind::Speeds => 3,
            TimingKind::Timesignatures | TimingKind::Combos => 2,
            _ => 1,
        };
        ensure!(
            self.values.len() == n,
            "{} requires {n} values",
            self.kind.tag()
        );
        match self.kind {
            TimingKind::Bpms => ensure!(
                self.values[0] > 0.,
                "BPM must be positive; legacy negative BPM files require normalization"
            ),
            TimingKind::Delays | TimingKind::Warps | TimingKind::Fakes => {
                ensure!(self.values[0] >= 0., "Duration must be nonnegative")
            }
            TimingKind::Timesignatures | TimingKind::Combos => ensure!(
                self.values.iter().all(|v| *v >= 1. && v.fract() == 0.),
                "Expected positive integers"
            ),
            TimingKind::Tickcounts => ensure!(
                self.values[0] >= 0. && self.values[0].fract() == 0.,
                "Tick count must be a nonnegative integer"
            ),
            TimingKind::Speeds => ensure!(
                self.values[1] >= 0. && [0., 1.].contains(&self.values[2]),
                "Speed transition requires duration and unit 0 (beats) or 1 (seconds)"
            ),
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timing {
    pub offset: f64,
    pub events: Vec<Event>,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            offset: 0.,
            events: vec![Event::value(TimingKind::Bpms, 0., 120.)],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Boundary {
    Before,
    Note,
    After,
}
#[derive(Clone, Debug, PartialEq)]
pub enum AudioPosition {
    Beat(f64),
    Delay { beat: f64 },
    Stop { beat: f64 },
    Warp { from: f64, to: f64 },
}
#[derive(Clone, Debug)]
pub struct Point {
    pub beat: f64,
    pub before: f64,
    pub note: f64,
    pub after: f64,
    pub bpm: f64,
    pub warped: bool,
    pub clamp: f64,
}
impl Timing {
    pub fn display_at(&self, note: f64, beat: f64, audio_seconds: f64, cmod: bool) -> f64 {
        if cmod {
            (self.seconds(note, Boundary::Note) - audio_seconds) * self.bpm(0.) / 60.
        } else {
            self.display_with_audio(note, beat, Some(audio_seconds), false)
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.offset.is_finite() && self.offset.abs() < 86400.,
            "Invalid offset"
        );
        ensure!(self.events.len() <= 100_000, "Too many timing events");
        let mut positions = BTreeSet::new();
        for event in &self.events {
            event.validate()?;
            ensure!(
                positions.insert((event.kind, event.beat.to_bits())),
                "Duplicate timing event"
            );
        }
        ensure!(
            self.events
                .iter()
                .any(|e| e.kind == TimingKind::Bpms && e.beat <= 0.),
            "Timing needs a BPM at or before beat zero"
        );
        Ok(())
    }
    pub fn from_tags(tags: &BTreeMap<String, String>, fallback: Option<&Self>) -> Result<Self> {
        // SSC split timing is a complete override, not a per-column merge.
        let split = tags
            .keys()
            .any(|k| TimingKind::parse(k).is_some() || k == "OFFSET");
        if !split {
            return Ok(fallback.cloned().unwrap_or_default());
        }
        let mut timing = Self {
            offset: tags
                .get("OFFSET")
                .map(|v| v.trim().parse())
                .transpose()?
                .unwrap_or_else(|| fallback.map_or(0., |t| t.offset)),
            events: Vec::new(),
        };
        for kind in TimingKind::ALL {
            if let Some(raw) = tags.get(kind.tag()) {
                for part in raw.split(',').filter(|v| !v.trim().is_empty()) {
                    let mut fields = part.trim().split('=');
                    let beat = fields
                        .next()
                        .context("Missing timing beat")?
                        .trim()
                        .parse()?;
                    let mut event = Event {
                        kind,
                        beat,
                        values: Vec::new(),
                        text: String::new(),
                    };
                    if kind == TimingKind::Labels {
                        event.text = fields.collect::<Vec<_>>().join("=");
                    } else {
                        event.values = fields
                            .map(|v| v.trim().parse())
                            .collect::<std::result::Result<_, _>>()?;
                        if kind == TimingKind::Combos && event.values.len() == 1 {
                            event.values.push(event.values[0]);
                        }
                    }
                    event.validate()?;
                    timing.events.push(event);
                }
            }
        }
        if !timing.events.iter().any(|e| e.kind == TimingKind::Bpms) {
            timing.events.push(Event::value(
                TimingKind::Bpms,
                0.,
                fallback.map_or(120., |t| t.bpm(0.)),
            ));
        }
        timing.validate()?;
        Ok(timing)
    }
    pub fn tags(&self) -> BTreeMap<String, String> {
        let mut result = BTreeMap::from([("OFFSET".into(), self.offset.to_string())]);
        for kind in TimingKind::ALL {
            let mut events: Vec<_> = self.events.iter().filter(|e| e.kind == kind).collect();
            events.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            let values = events
                .into_iter()
                .map(|e| {
                    format!(
                        "{}={}",
                        e.beat,
                        if kind == TimingKind::Labels {
                            e.text.clone()
                        } else {
                            e.values
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("=")
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            if !values.is_empty() {
                result.insert(kind.tag().into(), values);
            }
        }
        result
    }
    pub fn bpm(&self, beat: f64) -> f64 {
        self.events
            .iter()
            .filter(|e| e.kind == TimingKind::Bpms && e.beat <= beat)
            .max_by(|a, b| a.beat.total_cmp(&b.beat))
            .or_else(|| {
                self.events
                    .iter()
                    .filter(|e| e.kind == TimingKind::Bpms)
                    .min_by(|a, b| a.beat.total_cmp(&b.beat))
            })
            .map_or(120., |e| e.values[0])
    }
    pub fn points(&self) -> Vec<Point> {
        let mut beats = vec![0.];
        for e in &self.events {
            if matches!(
                e.kind,
                TimingKind::Bpms | TimingKind::Stops | TimingKind::Delays | TimingKind::Warps
            ) {
                beats.push(e.beat);
                if e.kind == TimingKind::Warps {
                    beats.push(e.beat + e.values[0]);
                }
            }
        }
        beats.sort_by(f64::total_cmp);
        beats.dedup();
        let first = beats[0];
        let mut time = first * 60. / self.bpm(first);
        let mut previous = first;
        let mut bpm = self.bpm(first);
        let mut end = f64::NEG_INFINITY;
        let mut high = f64::NEG_INFINITY;
        let mut out = Vec::new();
        for beat in beats {
            if previous >= end {
                time += (beat - previous) * 60. / bpm;
            }
            let before = time;
            for e in self
                .events
                .iter()
                .filter(|e| e.beat == beat && e.kind == TimingKind::Delays)
            {
                time += e.values[0];
            }
            let note = time;
            high = high.max(note);
            for e in self
                .events
                .iter()
                .filter(|e| e.beat == beat && e.kind == TimingKind::Stops)
            {
                time += e.values[0];
            }
            for e in self
                .events
                .iter()
                .filter(|e| e.beat == beat && e.kind == TimingKind::Warps)
            {
                end = end.max(beat + e.values[0]);
            }
            bpm = self.bpm(beat);
            out.push(Point {
                beat,
                before,
                note,
                after: time,
                bpm,
                warped: beat < end,
                clamp: high,
            });
            previous = beat;
        }
        // Offset is beat zero before any delay. Negative event history cancels
        // at the anchor rather than shifting the recording unexpectedly.
        let anchor = out.iter().find(|p| p.beat == 0.).map_or(0., |p| p.before) + self.offset;
        for p in &mut out {
            p.before -= anchor;
            p.note -= anchor;
            p.after -= anchor;
            p.clamp -= anchor;
        }
        out
    }
    pub fn seconds(&self, beat: f64, boundary: Boundary) -> f64 {
        let points = self.points();
        let p = points
            .iter()
            .rev()
            .find(|p| p.beat <= beat)
            .unwrap_or(&points[0]);
        if beat == p.beat {
            return match boundary {
                Boundary::Before => p.before,
                Boundary::Note => p.note,
                Boundary::After => p.after,
            };
        }
        p.after
            + if p.warped {
                0.
            } else {
                (beat - p.beat) * 60. / p.bpm
            }
    }
    pub fn position(&self, seconds: f64) -> AudioPosition {
        let points = self.points();
        for (i, p) in points.iter().enumerate() {
            if seconds >= p.before && seconds < p.note {
                return AudioPosition::Delay { beat: p.beat };
            }
            if seconds >= p.note && seconds < p.after {
                return AudioPosition::Stop { beat: p.beat };
            }
            if p.warped && (seconds - p.after).abs() < 1e-8 {
                let end = points[i..]
                    .iter()
                    .find(|v| !v.warped)
                    .map_or(p.beat, |v| v.beat);
                return AudioPosition::Warp {
                    from: p.beat,
                    to: end,
                };
            }
            if let Some(next) = points.get(i + 1)
                && seconds >= p.after
                && seconds < next.before
                && !p.warped
            {
                return AudioPosition::Beat(p.beat + (seconds - p.after) * p.bpm / 60.);
            }
        }
        let p = if seconds < points[0].before {
            &points[0]
        } else {
            points.last().unwrap()
        };
        AudioPosition::Beat(p.beat + (seconds - p.after) * p.bpm / 60.)
    }
    pub fn judgable(&self, beat: f64) -> bool {
        let points = self.points();
        let p = points
            .iter()
            .rev()
            .find(|p| p.beat <= beat)
            .unwrap_or(&points[0]);
        let fake = self
            .events
            .iter()
            .any(|e| e.kind == TimingKind::Fakes && beat >= e.beat && beat < e.beat + e.values[0]);
        let on_stop = self.events.iter().any(|e| {
            matches!(e.kind, TimingKind::Stops | TimingKind::Delays)
                && e.beat == beat
                && e.values[0] > 0.
        });
        !fake
            && (!p.warped || on_stop && p.beat == beat)
            && self.seconds(beat, Boundary::Note) + 1e-8 >= p.clamp
    }
    /// XMod display distance integrates SCROLLS; SPEEDS scales the complete
    /// field at the current clock. CMod deliberately ignores both.
    pub fn display(&self, note: f64, clock: f64, cmod: bool) -> f64 {
        self.display_with_audio(note, clock, None, cmod)
    }
    fn display_with_audio(
        &self,
        note: f64,
        clock: f64,
        audio_seconds: Option<f64>,
        cmod: bool,
    ) -> f64 {
        if cmod {
            return (self.seconds(note, Boundary::Note) - self.seconds(clock, Boundary::Note))
                * self.bpm(0.)
                / 60.;
        }
        let integral = |end: f64| {
            let (lo, hi, sign) = if end >= 0. {
                (0., end, 1.)
            } else {
                (end, 0., -1.)
            };
            let mut points = vec![lo, hi];
            points.extend(
                self.events
                    .iter()
                    .filter(|e| e.kind == TimingKind::Scrolls && e.beat > lo && e.beat < hi)
                    .map(|e| e.beat),
            );
            points.sort_by(f64::total_cmp);
            sign * points
                .windows(2)
                .map(|w| {
                    let value = self
                        .events
                        .iter()
                        .filter(|e| e.kind == TimingKind::Scrolls && e.beat <= w[0])
                        .max_by(|a, b| a.beat.total_cmp(&b.beat))
                        .map_or(1., |e| e.values[0]);
                    (w[1] - w[0]) * value
                })
                .sum::<f64>()
        };
        let mut speeds: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.kind == TimingKind::Speeds && e.beat <= clock)
            .collect();
        speeds.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        let mut speed = 1.;
        for (i, e) in speeds.iter().enumerate() {
            let elapsed = if e.values[2] == 0. {
                clock - e.beat
            } else {
                audio_seconds.unwrap_or_else(|| self.seconds(clock, Boundary::Note))
                    - self.seconds(e.beat, Boundary::Note)
            };
            let previous = if i == 0 { 1. } else { speeds[i - 1].values[0] };
            let ratio = if e.values[1] == 0. {
                1.
            } else {
                (elapsed / e.values[1]).clamp(0., 1.)
            };
            speed = previous + (e.values[0] - previous) * ratio;
        }
        (integral(note) - integral(clock)) * speed
    }
}
