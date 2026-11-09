//! Experimental, bounded MIDI/audio feedback. No lyrics enter the analysis.
//! Coverage and repeated-model agreement never establish pitch correctness.
use crate::{
    editor::{Project, TimeRange},
    timing::TempoMap,
    vocal_adaptive_planner::{
        self as plan, Frame, Limits, MidiPoint, Observation, Planner, Window,
    },
    vocal_generation::{Generation, PitchFrame, PitchSupport},
    vocal_pitch,
    vocal_pitch_analysis::{self, Analysis},
    vocal_section_analysis::{self as section, SupplementalSupport},
    vocals::{VocalChart, VocalTone},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const ALGORITHM: &str = "experimental-adaptive-vocal-review-v1";
/// Rebuild stateless inference sessions so temporary model allocations cannot
/// accumulate for the whole recursive job. No acoustic thresholds change.
pub const INFERENCE_BATCH: usize = 8;
pub fn implementation_hash() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("vocal_adaptive.rs"));
    h.update(include_bytes!("vocal_adaptive_planner.rs"));
    h.update(include_bytes!("vocal_section_analysis.rs"));
    h.update(crate::vocal_rmvpe::implementation_hash());
    format!("{:x}", h.finalize())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    pub pass: usize,
    pub start: f64,
    pub end: f64,
    pub before_midi: Option<u8>,
    pub after_midi: Option<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub algorithm: String,
    pub implementation_sha256: String,
    pub frame_count: usize,
    pub pcm_sha256: String,
    pub initial_residual_frames: usize,
    pub remaining_residual_frames: usize,
    pub scanned_frames: usize,
    pub budget_exhausted: bool,
    pub passes: Vec<plan::Pass>,
    pub alternatives: Vec<Alternative>,
    pub omitted_alternatives: usize,
}
impl Report {
    pub fn validate(&self, range: TimeRange, duration: f64) -> Result<()> {
        ensure!(
            self.algorithm == plan::ALGORITHM
                && self.implementation_sha256 == implementation_hash()
                && (1..=360001).contains(&self.frame_count)
                && ((self.frame_count - 1) as f64 * 0.01 - duration).abs() <= 0.0101
                && self.pcm_sha256.len() == 64
                && self.pcm_sha256.bytes().all(|c| c.is_ascii_hexdigit())
                && self.initial_residual_frames <= self.frame_count
                && self.remaining_residual_frames <= self.frame_count,
            "Adaptive report identity or source clock differs"
        );
        let limits = Limits::for_frames(self.frame_count);
        ensure!(
            self.passes.len() <= limits.maximum_calls
                && self.scanned_frames <= limits.maximum_scanned_frames,
            "Adaptive report exceeds processing budget"
        );
        let mut seen = std::collections::BTreeSet::new();
        let mut scanned = 0;
        for pass in &self.passes {
            let w = &pass.window;
            ensure!(
                seen.insert(w.id)
                    && w.start < w.end
                    && w.end <= self.frame_count
                    && w.context_start == w.start.saturating_sub(limits.context_frames)
                    && w.context_end == (w.end + limits.context_frames).min(self.frame_count)
                    && w.end - w.start <= limits.initial_frames
                    && w.depth <= limits.maximum_depth
                    && w.start as f64 * 0.01 < range.end
                    && w.end as f64 * 0.01 > range.start
                    && pass.before_residual_frames <= w.end - w.start
                    && pass.after_residual_frames <= w.end - w.start
                    && [
                        "resolved",
                        "unchanged_evidence",
                        "depth_limit",
                        "minimum_window",
                        "split_unresolved"
                    ]
                    .contains(&pass.reason.as_str()),
                "Invalid adaptive window receipt"
            );
            scanned += w.context_end - w.context_start;
        }
        ensure!(
            scanned == self.scanned_frames,
            "Adaptive scan accounting differs"
        );
        ensure!(
            self.alternatives.len() <= 2000
                && self.alternatives.iter().all(|a| seen.contains(&a.pass)
                    && a.start.is_finite()
                    && a.end.is_finite()
                    && a.start >= 0.
                    && a.start < a.end
                    && a.end <= duration + 0.01
                    && a.before_midi.is_none_or(|p| (36..=84).contains(&p))
                    && a.after_midi.is_none_or(|p| (36..=84).contains(&p))
                    && a.before_midi != a.after_midi),
            "Invalid retained pitch alternative"
        );
        Ok(())
    }
}
fn pitch(f: &vocal_pitch::Frame) -> Option<f64> {
    f.frequency_hz
        .filter(|_| f.confidence >= 0.1 && f.rms > 0.)
        .map(|hz| 69. + 12. * (hz / 440.).log2())
        .filter(|p| (35.5..84.5).contains(p))
}
fn raster(chart: &VocalChart, timing: &TempoMap, n: usize, range: TimeRange) -> Vec<MidiPoint> {
    let mut cursor = 0;
    (0..n)
        .map(|i| {
            let t = i as f64 * 0.01;
            while cursor < chart.notes.len() && timing.seconds(chart.notes[cursor].end) <= t {
                cursor += 1;
            }
            let note = chart
                .notes
                .get(cursor)
                .filter(|n| timing.seconds(n.start) <= t && t >= range.start && t < range.end);
            MidiPoint {
                occupied: note.is_some(),
                pitch: note.and_then(|n| {
                    if let VocalTone::Pitched { midi } = n.tone {
                        Some(midi as f64)
                    } else {
                        None
                    }
                }),
            }
        })
        .collect()
}
/// A direct, fractional-lag test at the proposed period, its octave above and
/// its octave below. It measures periodicity, not voice/singer identity.
fn physical(pcm: &[f32], index: usize, midi: f64) -> [f64; 3] {
    let mut x = [0f64; 1536];
    let center = index as isize * 160;
    for (i, v) in x.iter_mut().enumerate() {
        let at = center + i as isize - 768;
        if at >= 0 && (at as usize) < pcm.len() {
            *v = pcm[at as usize] as f64;
        }
    }
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    for v in &mut x {
        *v -= mean;
    }
    let corr = |lag: f64| {
        let n = lag.floor() as usize;
        let frac = lag - n as f64;
        if n == 0 || n + 2 >= x.len() {
            return 0.;
        }
        let (mut dot, mut a, mut b) = (0., 0., 0.);
        for i in 0..x.len() - n - 1 {
            let y = x[i + n] * (1. - frac) + x[i + n + 1] * frac;
            dot += x[i] * y;
            a += x[i] * x[i];
            b += y * y;
        }
        if a * b > 1e-20 {
            (dot / (a * b).sqrt()).clamp(-1., 1.)
        } else {
            0.
        }
    };
    let period = 16000. / (440. * 2f64.powf((midi - 69.) / 12.));
    let full = corr(period);
    [full, full - corr(period / 2.), corr(period * 2.) - full]
}
fn decoded(raw: &[vocal_pitch::Frame]) -> Result<Vec<PitchFrame>> {
    let mut frames = raw
        .iter()
        .map(|f| PitchFrame {
            time_seconds: f.time_seconds,
            midi: f
                .frequency_hz
                .filter(|_| f.confidence > 0.03)
                .map(|hz| 69. + 12. * (hz / 440.).log2())
                .filter(|p| (35.5..84.5).contains(p)),
            yin_midi: None,
            mcleod_midi: None,
            clarity: f.confidence as f64,
            rms: f.rms,
            local_rms: Some(f.rms),
            local_energy_active: None,
            support: PitchSupport::ContourPath,
        })
        .collect::<Vec<_>>();
    let weights = raw
        .iter()
        .map(|f| (f.confidence as f64).max(0.1))
        .collect::<Vec<_>>();
    crate::vocal_pitch_tracking::weighted_note_path(&mut frames, 3.2, &weights, &|| false)?;
    Ok(frames)
}
pub struct Review<'a> {
    project: &'a Project,
    timing: &'a TempoMap,
    original: Analysis,
    generation: Generation,
    pcm: Vec<f32>,
    planner: Planner,
    contexts: Vec<(Option<f64>, usize, usize, usize)>,
    support: Vec<SupplementalSupport>,
    initial: usize,
    range: TimeRange,
    pending: Option<Window>,
    alternatives: Vec<Alternative>,
    omitted_alternatives: usize,
}
impl<'a> Review<'a> {
    /// Sources are decoded in full. Only cores are replaced; inference retains
    /// surrounding original audio. Stereo energies cannot cancel to silence.
    pub fn new(
        project: &'a Project,
        timing: &'a TempoMap,
        generation: &Generation,
        range: Option<TimeRange>,
        pcm: Vec<f32>,
        source_stereo: &[f32],
        stem_stereo: &[f32],
    ) -> Result<Self> {
        let original = vocal_pitch_analysis::propose(project, timing, generation, range)?;
        let range = original.range;
        let raw = generation.rmvpe_pitch.as_ref().unwrap();
        let fresh = crate::vocal_rmvpe::finish_pcm(&pcm, raw.frames.clone())?;
        ensure!(
            fresh.pcm_sha256 == raw.pcm_sha256
                && source_stereo.len() == stem_stereo.len()
                && source_stereo.len().is_multiple_of(2)
                && source_stereo.len() / 2 == pcm.len()
                && source_stereo
                    .iter()
                    .chain(stem_stereo)
                    .all(|v| v.is_finite()),
            "Adaptive PCM identity or stereo clock differs"
        );
        let n = raw.frames.len();
        let chart = project
            .content
            .vocals
            .as_ref()
            .context("No current vocal chart")?;
        let midi = raster(chart, timing, n, range);
        let mut frames = Vec::with_capacity(n);
        for (i, (r, m)) in raw.frames.iter().zip(midi).enumerate() {
            let a = (i * 160).saturating_sub(80).min(pcm.len()) * 2;
            let b = (i * 160 + 80).min(pcm.len()) * 2;
            let in_range = r.time_seconds >= range.start && r.time_seconds < range.end;
            let energy = |s: &[f32]| {
                if in_range && b > a {
                    plan::stereo_energy(&s[a..b])
                } else {
                    Ok(0.)
                }
            };
            frames.push(Frame {
                seconds: r.time_seconds,
                observed_midi: if in_range { pitch(r) } else { None },
                confidence: if in_range { r.confidence as f64 } else { 0. },
                source_energy: energy(source_stereo)?,
                stem_energy: energy(stem_stereo)?,
                midi: m.pitch,
                occupied: m.occupied,
            });
        }
        let planner = Planner::new(frames, Limits::for_frames(n))?;
        let initial = planner.residual_frames();
        let contexts = raw
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let a = i / 3000 * 3000;
                (
                    pitch(f),
                    a,
                    (a + 3000).min(n),
                    usize::from(pitch(f).is_some()),
                )
            })
            .collect();
        Ok(Self {
            project,
            timing,
            original,
            generation: generation.clone(),
            pcm,
            planner,
            contexts,
            support: vec![SupplementalSupport::default(); n],
            initial,
            range,
            pending: None,
            alternatives: vec![],
            omitted_alternatives: 0,
        })
    }
    pub fn pcm(&self) -> &[f32] {
        &self.pcm
    }
    pub fn next_window(&mut self) -> Result<Option<Window>> {
        let next = self.planner.next_window()?;
        self.pending = next.clone();
        Ok(next)
    }
    pub fn pass_count(&self) -> usize {
        self.planner.passes.len()
    }
    pub fn complete(&mut self, w: &Window, observations: Vec<vocal_pitch::Frame>) -> Result<()> {
        ensure!(
            self.pending.as_ref() == Some(w),
            "Rescan window differs from pending work"
        );
        ensure!(
            observations.len() == w.end - w.start
                && observations.iter().enumerate().all(|(j, f)| (f.time_seconds
                    - (w.start + j) as f64 * 0.01)
                    .abs()
                    < 1e-9
                    && f.confidence.is_finite()
                    && (0. ..=1.).contains(&f.confidence)
                    && f.rms.is_finite()
                    && f.rms >= 0.
                    && f.frequency_hz
                        .is_none_or(|p| p.is_finite() && (20. ..=2200.).contains(&p))),
            "Invalid adaptive rescan"
        );
        let raw = &mut self.generation.rmvpe_pitch.as_mut().unwrap().frames;
        let pair = |old: &vocal_pitch::Frame, new: &vocal_pitch::Frame| {
            let a = pitch(old).map(|p| p.round().clamp(36., 84.) as u8);
            let b = pitch(new).map(|p| p.round().clamp(36., 84.) as u8);
            (a != b).then_some((a, b))
        };
        let mut j = 0;
        while j < observations.len() {
            let Some((a, b)) = pair(&raw[w.start + j], &observations[j]) else {
                j += 1;
                continue;
            };
            let first = j;
            j += 1;
            while j < observations.len()
                && pair(&raw[w.start + j], &observations[j]) == Some((a, b))
            {
                j += 1;
            }
            if j - first >= 4 {
                if self.alternatives.len() < 2000 {
                    self.alternatives.push(Alternative {
                        pass: w.id,
                        start: (w.start + first) as f64 * 0.01,
                        end: ((w.start + j) as f64 * 0.01).min(self.project.duration),
                        before_midi: a,
                        after_midi: b,
                    });
                } else {
                    self.omitted_alternatives += 1;
                }
            }
        }
        raw[w.start..w.end].clone_from_slice(&observations);
        let frames = decoded(raw)?;
        for (j, new) in observations.iter().enumerate() {
            let i = w.start + j;
            let p = pitch(new);
            let old = self.contexts[i];
            let different = (old.1, old.2) != (w.start, w.end);
            let count = if p.is_some() && old.0.zip(p).is_some_and(|(a, b)| (a - b).abs() <= 0.5) {
                (old.3 + usize::from(different)).min(2)
            } else {
                usize::from(p.is_some())
            };
            self.contexts[i] = (p, w.start, w.end, count);
        }
        // Decode across the full clock, rather than independently segmenting
        // every window. A sustained note may cross any number of chunk edges.
        for (i, f) in frames.iter().enumerate() {
            if let Some(p) = f.midi.map(f64::round) {
                let old = self.support[i];
                let values = if old.midi == Some(p) {
                    [old.periodicity, old.half_margin, old.double_advantage]
                } else {
                    physical(&self.pcm, i, p)
                };
                self.support[i] = SupplementalSupport {
                    midi: Some(p),
                    periodicity: values[0],
                    half_margin: values[1],
                    double_advantage: values[2],
                    agreeing_contexts: self.contexts[i].3,
                };
            } else {
                self.support[i] = SupplementalSupport::default();
            }
        }
        let chart = self.project.content.vocals.as_ref().unwrap();
        let section = section::compare_adaptive(
            chart,
            self.timing,
            &self.generation,
            &frames,
            self.range,
            &self.support,
        )?;
        // Always derive the working chart from the original. A later disputed
        // observation retracts a draft addition without deleting authored notes.
        let working = section::apply(chart, &section.repairs)?;
        let midi = raster(&working, self.timing, frames.len(), self.range);
        let o = observations
            .iter()
            .map(|f| Observation {
                midi: pitch(f),
                confidence: f.confidence as f64,
            })
            .collect::<Vec<_>>();
        self.planner.complete(&o, Some(&midi))?;
        self.pending = None;
        Ok(())
    }
    pub fn finish(mut self) -> Result<Analysis> {
        ensure!(
            self.next_window()?.is_none(),
            "Adaptive scan has pending work"
        );
        let frames = decoded(&self.generation.rmvpe_pitch.as_ref().unwrap().frames)?;
        let chart = self.project.content.vocals.as_ref().unwrap();
        self.original.section = section::compare_adaptive(
            chart,
            self.timing,
            &self.generation,
            &frames,
            self.range,
            &self.support,
        )?;
        // Retain unresolved sounds, wrong-pitch alternatives and unsupported
        // MIDI as findings. None are silently filled or automatically deleted.
        let residuals = self.planner.frames();
        let mut i = 0;
        while i < residuals.len() && self.original.section.findings.len() < 2000 {
            let kind = plan::residual(&residuals[i]);
            if kind.is_none() {
                i += 1;
                continue;
            }
            let a = i;
            i += 1;
            while i < residuals.len() && plan::residual(&residuals[i]) == kind {
                i += 1;
            }
            if i - a < 4 {
                continue;
            }
            let detail = match kind.unwrap() {
                plan::Residual::MissingPitch => {
                    "Uncharted vocal pitch remains after smaller-context rescans; insufficient support for an automatic repair."
                }
                plan::Residual::PitchMismatch => {
                    "MIDI and the rescanned vocal pitch disagree. Review the contour, octave and singer; the original note is retained."
                }
                plan::Residual::UnsupportedMidi => {
                    "MIDI has a note but the vocal detector has no supported pitch. Quiet or missed singing remains possible; the note is retained."
                }
                plan::Residual::ProvenSilence => {
                    "MIDI has a note over exact decoded stereo silence in both sources. Review this extra note; it is retained."
                }
                plan::Residual::UnpitchedSound => {
                    "Vocal-stem sound remains without a reliable pitch or MIDI note. This may be breath, consonants, rap, leakage or missed singing."
                }
            };
            self.original.section.findings.push(section::Finding {
                start: a as f64 * 0.01,
                end: (i as f64 * 0.01).min(self.range.end),
                detail: detail.into(),
            });
        }
        let report = Report {
            algorithm: plan::ALGORITHM.into(),
            implementation_sha256: implementation_hash(),
            frame_count: residuals.len(),
            pcm_sha256: self
                .generation
                .rmvpe_pitch
                .as_ref()
                .unwrap()
                .pcm_sha256
                .clone(),
            initial_residual_frames: self.initial,
            remaining_residual_frames: self.planner.residual_frames(),
            scanned_frames: self.planner.scanned_frames(),
            budget_exhausted: self.planner.budget_exhausted,
            passes: self.planner.passes,
            alternatives: self.alternatives,
            omitted_alternatives: self.omitted_alternatives,
        };
        report.validate(self.range, self.project.duration)?;
        self.original.generation_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &self.generation.rmvpe_pitch,
                &report
            ))?)
        );
        self.original.algorithm = ALGORITHM.into();
        self.original.adaptive = Some(Box::new(report));
        self.original.validate(self.project, self.timing)?;
        Ok(self.original)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodicity_rejects_octave_errors_and_silence() {
        let pcm = (0..16000)
            .map(|i| (std::f64::consts::TAU * 220. * i as f64 / 16000.).sin() as f32)
            .collect::<Vec<_>>();
        let correct = physical(&pcm, 50, 57.);
        let low = physical(&pcm, 50, 45.);
        let high = physical(&pcm, 50, 69.);
        assert!(correct[0] > 0.99 && correct[1] > 1.9);
        assert!(low[1].abs() < 0.01);
        assert!(high[0] < 0.);
        assert_eq!(physical(&vec![0.; 16000], 50, 57.), [0.; 3]);
    }
}
