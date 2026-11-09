//! Bounded audio/MIDI feedback planning. Residual coverage is not pitch truth.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
pub const ALGORITHM: &str = "adaptive-midi-audio-feedback-v1;100Hz;6s-cores-1s-context;overlapping-halves;minimum250ms;depth6;repeat-evidence-stop2-below1s;no-lyric-input";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub seconds: f64,
    pub observed_midi: Option<f64>,
    pub confidence: f64,
    pub source_energy: f64,
    pub stem_energy: f64,
    pub midi: Option<f64>,
    pub occupied: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Residual {
    MissingPitch,
    PitchMismatch,
    UnsupportedMidi,
    ProvenSilence,
    UnpitchedSound,
}
pub fn residual(f: &Frame) -> Option<Residual> {
    let observed = f
        .observed_midi
        .filter(|_| f.confidence >= 0.1 && f.stem_energy > 0.);
    if f.occupied && f.source_energy == 0. && f.stem_energy == 0. {
        return Some(Residual::ProvenSilence);
    }
    match (f.occupied, f.midi, observed) {
        (false, _, Some(_)) => Some(Residual::MissingPitch),
        (true, Some(a), Some(b)) if (a - b).abs() > 0.75 => Some(Residual::PitchMismatch),
        (true, Some(_), None) => Some(Residual::UnsupportedMidi),
        (false, _, None) if f.stem_energy >= 0.0015 * 0.0015 => Some(Residual::UnpitchedSound),
        _ => None,
    }
}
/// Squared channel energy avoids mistaking opposite stereo phases for silence.
pub fn stereo_energy(pcm: &[f32]) -> Result<f64> {
    ensure!(
        !pcm.is_empty() && pcm.len().is_multiple_of(2) && pcm.iter().all(|p| p.is_finite()),
        "Finite stereo samples required"
    );
    Ok(pcm.iter().map(|p| (*p as f64).powi(2)).sum::<f64>() / pcm.len() as f64)
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Limits {
    pub initial_frames: usize,
    pub context_frames: usize,
    pub minimum_frames: usize,
    pub maximum_depth: usize,
    pub maximum_calls: usize,
    pub maximum_scanned_frames: usize,
    pub unchanged_limit: usize,
}
impl Limits {
    pub fn for_frames(n: usize) -> Self {
        Self {
            initial_frames: 600,
            context_frames: 100,
            minimum_frames: 25,
            maximum_depth: 6,
            maximum_calls: 1024,
            maximum_scanned_frames: n.saturating_mul(12).saturating_add(1600),
            unchanged_limit: 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub id: usize,
    pub depth: usize,
    pub start: usize,
    pub end: usize,
    pub context_start: usize,
    pub context_end: usize,
    pub unchanged_ancestors: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pass {
    pub window: Window,
    pub before_residual_frames: usize,
    pub after_residual_frames: usize,
    pub audio_pitch_changed: bool,
    pub reason: String,
}
#[derive(Clone, Copy)]
pub struct Observation {
    pub midi: Option<f64>,
    pub confidence: f64,
}
#[derive(Clone, Copy)]
pub struct MidiPoint {
    pub pitch: Option<f64>,
    pub occupied: bool,
}
#[derive(Clone)]
struct Pending {
    window: Window,
    before: usize,
    signature: Vec<Option<i32>>,
}
pub struct Planner {
    frames: Vec<Frame>,
    limits: Limits,
    queue: VecDeque<Window>,
    seen: BTreeSet<(usize, usize, usize)>,
    pending: Option<Pending>,
    next_id: usize,
    scanned: usize,
    pub passes: Vec<Pass>,
    pub budget_exhausted: bool,
}
fn valid_pitch(p: Option<f64>) -> bool {
    p.is_none_or(|p| p.is_finite() && (0. ..=127.).contains(&p))
}
fn marked(frames: &[Frame]) -> Vec<bool> {
    let raw = frames
        .iter()
        .map(|f| residual(f).is_some())
        .collect::<Vec<_>>();
    let mut out = vec![false; raw.len()];
    let mut i = 0;
    while i < raw.len() {
        if !raw[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < raw.len() && raw[i] {
            i += 1;
        }
        if i - a >= 4 {
            out[a..i].fill(true);
        }
    }
    out
}
fn signature(frames: &[Frame]) -> Vec<Option<i32>> {
    frames
        .iter()
        .map(|f| {
            f.observed_midi
                .filter(|_| f.confidence >= 0.1 && f.stem_energy > 0.)
                .map(|p| (p * 4.).round() as i32)
        })
        .collect()
}
impl Planner {
    pub fn new(frames: Vec<Frame>, limits: Limits) -> Result<Self> {
        ensure!(
            !frames.is_empty()
                && frames.len() <= 360001
                && frames.iter().enumerate().all(|(i, f)| f.seconds.is_finite()
                    && (f.seconds - i as f64 * 0.01).abs() < 1e-9
                    && valid_pitch(f.observed_midi)
                    && valid_pitch(f.midi)
                    && f.confidence.is_finite()
                    && (0. ..=1.).contains(&f.confidence)
                    && f.source_energy.is_finite()
                    && f.source_energy >= 0.
                    && f.stem_energy.is_finite()
                    && f.stem_energy >= 0.
                    && (f.occupied || f.midi.is_none())),
            "Invalid source-clock feedback frames"
        );
        ensure!(
            limits.minimum_frames >= 10
                && limits.initial_frames >= limits.minimum_frames
                && limits.initial_frames <= 3000
                && limits.context_frames <= 300
                && limits.maximum_depth <= 8
                && (1..=1024).contains(&limits.maximum_calls)
                && limits.maximum_scanned_frames > 0
                && (1..=4).contains(&limits.unchanged_limit),
            "Invalid adaptive processing limits"
        );
        let flags = marked(&frames);
        let mut plan = Self {
            frames,
            limits,
            queue: VecDeque::new(),
            seen: BTreeSet::new(),
            pending: None,
            next_id: 0,
            scanned: 0,
            passes: vec![],
            budget_exhausted: false,
        };
        for a in (0..flags.len()).step_by(limits.initial_frames) {
            let b = (a + limits.initial_frames).min(flags.len());
            if flags[a..b].iter().any(|b| *b) {
                plan.push(a, b, 0, 0);
            }
        }
        Ok(plan)
    }
    fn push(&mut self, a: usize, b: usize, depth: usize, unchanged: usize) {
        if a >= b || !self.seen.insert((a, b, depth)) {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push_back(Window {
            id,
            depth,
            start: a,
            end: b,
            context_start: a.saturating_sub(self.limits.context_frames),
            context_end: (b + self.limits.context_frames).min(self.frames.len()),
            unchanged_ancestors: unchanged,
        });
    }
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }
    pub fn next_window(&mut self) -> Result<Option<Window>> {
        ensure!(
            self.pending.is_none(),
            "Complete the current adaptive window first"
        );
        while let Some(w) = self.queue.pop_front() {
            let count = w.context_end - w.context_start;
            if self.passes.len() >= self.limits.maximum_calls
                || self.scanned + count > self.limits.maximum_scanned_frames
            {
                self.budget_exhausted = true;
                self.queue.push_front(w);
                return Ok(None);
            }
            let flags = marked(&self.frames);
            let before = flags[w.start..w.end].iter().filter(|b| **b).count();
            if before == 0 {
                continue;
            }
            self.scanned += count;
            self.pending = Some(Pending {
                before,
                signature: signature(&self.frames[w.start..w.end]),
                window: w.clone(),
            });
            return Ok(Some(w));
        }
        Ok(None)
    }
    /// Caller supplies verified source-bound observations and its working MIDI.
    /// All values are checked before state changes; no project is written here.
    pub fn complete(
        &mut self,
        observations: &[Observation],
        midi: Option<&[MidiPoint]>,
    ) -> Result<()> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active adaptive window"))?;
        let w = &pending.window;
        ensure!(
            observations.len() == w.end - w.start
                && observations.iter().all(|o| valid_pitch(o.midi)
                    && o.confidence.is_finite()
                    && (0. ..=1.).contains(&o.confidence))
                && midi.is_none_or(|m| m.len() == self.frames.len()
                    && m.iter().all(|m| valid_pitch(m.pitch)
                        && (!m.occupied && m.pitch.is_none() || m.occupied))),
            "Invalid adaptive observation or MIDI update"
        );
        let pending = self.pending.take().unwrap();
        let w = pending.window;
        for (f, o) in self.frames[w.start..w.end].iter_mut().zip(observations) {
            f.observed_midi = o.midi;
            f.confidence = o.confidence;
        }
        if let Some(midi) = midi {
            for (f, m) in self.frames.iter_mut().zip(midi) {
                f.midi = m.pitch;
                f.occupied = m.occupied;
            }
        }
        let after = marked(&self.frames)[w.start..w.end]
            .iter()
            .filter(|b| **b)
            .count();
        let changed = signature(&self.frames[w.start..w.end]) != pending.signature;
        let unchanged = if !changed && after >= pending.before {
            w.unchanged_ancestors + 1
        } else {
            0
        };
        let reason = if after == 0 {
            "resolved"
        } else if unchanged >= self.limits.unchanged_limit && w.end - w.start <= 100 {
            "unchanged_evidence"
        } else if w.depth >= self.limits.maximum_depth {
            "depth_limit"
        } else if w.end - w.start < self.limits.minimum_frames * 2 {
            "minimum_window"
        } else {
            "split_unresolved"
        };
        if reason == "split_unresolved" {
            let middle = (w.start + w.end) / 2;
            let overlap = ((w.end - w.start) / 8).min(10);
            let left = (middle + overlap).min(w.end - 1);
            let right = middle.saturating_sub(overlap).max(w.start + 1);
            self.push(w.start, left, w.depth + 1, unchanged);
            self.push(right, w.end, w.depth + 1, unchanged);
        }
        self.passes.push(Pass {
            window: w,
            before_residual_frames: pending.before,
            after_residual_frames: after,
            audio_pitch_changed: changed,
            reason: reason.into(),
        });
        Ok(())
    }
    pub fn residual_frames(&self) -> usize {
        marked(&self.frames).into_iter().filter(|b| *b).count()
    }
    pub fn scanned_frames(&self) -> usize {
        self.scanned
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frames(n: usize) -> Vec<Frame> {
        (0..n)
            .map(|i| Frame {
                seconds: i as f64 * 0.01,
                observed_midi: None,
                confidence: 0.,
                source_energy: 0.,
                stem_energy: 0.,
                midi: None,
                occupied: false,
            })
            .collect()
    }
    #[test]
    fn short_sounds_crossing_chunk_edges_are_found_before_partitioning() {
        let mut f = frames(1200);
        for f in &mut f[597..605] {
            f.observed_midi = Some(60.);
            f.confidence = 0.9;
            f.source_energy = 0.1;
            f.stem_energy = 0.1;
        }
        let mut p = Planner::new(f, Limits::for_frames(1200)).unwrap();
        let a = p.next_window().unwrap().unwrap();
        assert_eq!((a.start, a.end), (0, 600));
        let old = p.frames()[a.start..a.end]
            .iter()
            .map(|f| Observation {
                midi: f.observed_midi,
                confidence: f.confidence,
            })
            .collect::<Vec<_>>();
        p.complete(&old, None).unwrap();
        let b = p.next_window().unwrap().unwrap();
        assert_eq!((b.start, b.end), (600, 1200));
    }
    #[test]
    fn unchanged_evidence_stops_recursion_and_remains_unresolved() {
        let mut f = frames(600);
        for f in &mut f {
            f.observed_midi = Some(60.);
            f.confidence = 0.9;
            f.source_energy = 0.1;
            f.stem_energy = 0.1;
        }
        let mut p = Planner::new(f, Limits::for_frames(600)).unwrap();
        let mut calls = 0;
        while let Some(w) = p.next_window().unwrap() {
            let o = p.frames()[w.start..w.end]
                .iter()
                .map(|f| Observation {
                    midi: f.observed_midi,
                    confidence: f.confidence,
                })
                .collect::<Vec<_>>();
            p.complete(&o, None).unwrap();
            calls += 1;
            assert!(calls <= 15);
        }
        assert_eq!(calls, 15);
        assert_eq!(p.residual_frames(), 600);
        assert!(p.passes.iter().any(|p| p.reason == "unchanged_evidence"));
    }
    #[test]
    fn supported_midi_update_resolves_coverage_without_more_scans() {
        let mut f = frames(100);
        for f in &mut f {
            f.observed_midi = Some(60.);
            f.confidence = 0.9;
            f.source_energy = 0.1;
            f.stem_energy = 0.1;
        }
        let mut p = Planner::new(f, Limits::for_frames(100)).unwrap();
        let w = p.next_window().unwrap().unwrap();
        let o = vec![
            Observation {
                midi: Some(60.),
                confidence: 0.9
            };
            w.end - w.start
        ];
        let m = vec![
            MidiPoint {
                pitch: Some(60.),
                occupied: true
            };
            100
        ];
        p.complete(&o, Some(&m)).unwrap();
        assert!(p.next_window().unwrap().is_none());
        assert_eq!(p.residual_frames(), 0);
    }
    #[test]
    fn missing_model_pitch_and_counterphase_are_not_proof_of_silence() {
        let mut f = frames(1).remove(0);
        f.midi = Some(60.);
        f.occupied = true;
        f.source_energy = 1e-12;
        assert_eq!(residual(&f), Some(Residual::UnsupportedMidi));
        let energy = stereo_energy(&[1., -1., -1., 1.]).unwrap();
        assert_eq!(energy, 1.);
        f.source_energy = energy;
        f.stem_energy = energy;
        assert_eq!(residual(&f), Some(Residual::UnsupportedMidi));
        f.source_energy = 0.;
        f.stem_energy = 0.;
        assert_eq!(residual(&f), Some(Residual::ProvenSilence));
    }
    #[test]
    fn invalid_updates_are_atomic_and_budgets_are_bounded() {
        let mut f = frames(100);
        for f in &mut f {
            f.stem_energy = 0.1;
        }
        let mut limits = Limits::for_frames(100);
        limits.maximum_calls = 1;
        let mut p = Planner::new(f, limits).unwrap();
        let w = p.next_window().unwrap().unwrap();
        let before = serde_json::to_vec(p.frames()).unwrap();
        assert!(
            p.complete(
                &[Observation {
                    midi: Some(f64::NAN),
                    confidence: 1.
                }],
                None
            )
            .is_err()
        );
        assert_eq!(before, serde_json::to_vec(p.frames()).unwrap());
        p.complete(
            &vec![
                Observation {
                    midi: None,
                    confidence: 0.
                };
                w.end - w.start
            ],
            None,
        )
        .unwrap();
        assert!(p.next_window().unwrap().is_none());
        assert!(p.budget_exhausted);
    }
}
