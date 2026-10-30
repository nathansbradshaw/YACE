//! Propose vocal notes from the supplied recording and timed words.
//! Keep uncertain pitches flagged. Missing words stay unknown; uncertain pitch
//! does not turn a sung note into an unpitched talkie.
use crate::{
    clean::{AlignedWord, StemAlignment, WordTimings},
    models,
    timing::TempoMap,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use anyhow::{Result, ensure};
use pitch_detection::detector::{PitchDetector, mcleod::McLeodDetector, yin::YINDetector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

const RATE: usize = 22050;
const WINDOW: usize = 2048;
const HOP: usize = 220;
pub const ALGORITHM: &str = "rust-yin-mcleod-v7; pitch-first-missing-lyrics; recognition-preserving-ordered-lyrics; 22050Hz; centered2048; hop220; consensus0.55semitone; optional-source-hop-energy-gate; anchored-YIN-recovery<=0.8s,period-correlation>=0.85,known-word,stable-dual-anchors; optional-neural-YIN-recovery-score0.6,agreement0.5semi,period0.75,nonharmonic-margin0.15,fundamental-contrast0.15; optional-SwiftF0-score0.8,word,local-energy,pairwise-DSP-veto0.55semi,stable3of5within0.7semi,period0.75,fundamental-contrast0.15; median5; no-transposition; source-time; same-word-slide-only-contiguous<=1tick; explicit-continue-word-across-gaps; short-word-fragment-and-coverage-boundary-flags";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "PolicyWire")]
pub struct Policy {
    pub minimum_note_seconds: f64,
    pub phrase_gap_seconds: f64,
    pub maximum_phrase_seconds: f64,
    pub absolute_rms_floor: f64,
    pub relative_rms_floor: f64,
    /// Older reports omitted this field and therefore retain strict consensus.
    #[serde(default)]
    pub recover_bounded_yin: bool,
    /// Require energy around the actual frame timestamp, so a long analysis
    /// window cannot label neighboring silence as a sung pitch.
    #[serde(default)]
    pub local_energy_gate: bool,
}
/// Accept old strict reports without retaining experimental runtime switches.
/// Reports that actually enabled rejected experiments require their frozen
/// research runner; silently interpreting them as another policy is unsafe.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWire {
    minimum_note_seconds: f64,
    phrase_gap_seconds: f64,
    maximum_phrase_seconds: f64,
    absolute_rms_floor: f64,
    relative_rms_floor: f64,
    #[serde(default)]
    recover_bounded_yin: bool,
    #[serde(default)]
    local_energy_gate: bool,
    #[serde(default)]
    recover_contour_tails: bool,
    #[serde(default)]
    recover_short_consensus: bool,
}
impl TryFrom<PolicyWire> for Policy {
    type Error = &'static str;
    fn try_from(w: PolicyWire) -> std::result::Result<Self, Self::Error> {
        if w.recover_contour_tails || w.recover_short_consensus {
            return Err("Rejected experimental vocal policy requires its frozen research runner");
        }
        Ok(Self {
            minimum_note_seconds: w.minimum_note_seconds,
            phrase_gap_seconds: w.phrase_gap_seconds,
            maximum_phrase_seconds: w.maximum_phrase_seconds,
            absolute_rms_floor: w.absolute_rms_floor,
            relative_rms_floor: w.relative_rms_floor,
            recover_bounded_yin: w.recover_bounded_yin,
            local_energy_gate: w.local_energy_gate,
        })
    }
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            minimum_note_seconds: 0.080,
            phrase_gap_seconds: 0.65,
            maximum_phrase_seconds: 12.,
            absolute_rms_floor: 0.0015,
            relative_rms_floor: 0.015,
            recover_bounded_yin: true,
            local_energy_gate: true,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.minimum_note_seconds.is_finite()
                && (0.030..=0.5).contains(&self.minimum_note_seconds),
            "Invalid vocal minimum note duration"
        );
        ensure!(
            self.phrase_gap_seconds.is_finite() && (0.15..=3.).contains(&self.phrase_gap_seconds),
            "Invalid vocal breath gap"
        );
        ensure!(
            self.maximum_phrase_seconds.is_finite()
                && (2.0..=30.).contains(&self.maximum_phrase_seconds),
            "Invalid maximum phrase duration"
        );
        ensure!(
            self.absolute_rms_floor.is_finite()
                && (0.00001..=0.1).contains(&self.absolute_rms_floor),
            "Invalid vocal RMS floor"
        );
        ensure!(
            self.relative_rms_floor.is_finite() && (0.001..=0.2).contains(&self.relative_rms_floor),
            "Invalid relative vocal RMS floor"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LyricSource {
    pub text: String,
    pub source: String,
    /// Optional provider line timing. A line timestamp never becomes a word
    /// timestamp: it can only constrain an acoustic alignment search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_lyrics: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    ReviewRequired,
    NeedsLyrics,
    NoVoicedPitch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    MissingLyrics,
    UntimedWord,
    WordWithoutPitch,
    PitchDisagreement,
    OutOfRangePitch,
    WordLevelAlignment,
    PhraseBoundary,
    UnsupportedPitchGap,
    ShortWordFragment,
    CoveragePhraseBoundary,
    LyricDisagreement,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub reason: Reason,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchFrame {
    pub time_seconds: f64,
    pub midi: Option<f64>,
    #[serde(default)]
    pub yin_midi: Option<f64>,
    #[serde(default)]
    pub mcleod_midi: Option<f64>,
    /// Detector agreement/clarity, not a calibrated correctness probability.
    pub clarity: f64,
    pub rms: f64,
    #[serde(default)]
    pub local_rms: Option<f64>,
    /// None for legacy/disabled policies; false blocks all recovery as well
    /// as original consensus. Raw detector estimates remain diagnostic data.
    #[serde(default)]
    pub local_energy_active: Option<bool>,
    #[serde(default)]
    pub support: PitchSupport,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PitchSupport {
    #[default]
    Unspecified,
    Consensus,
    AnchoredYin,
    NeuralYin,
    SwiftF0,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeuralPitchEvidence {
    pub audio_sha256: String,
    pub vocal_stem_sha256: String,
    pub duration_seconds: f64,
    pub model_sha256: String,
    pub implementation_sha256: String,
    /// Raw polyphonic neural proposals, not approved lead-vocal notes.
    pub notes: Vec<crate::mapping::PitchNote>,
}
pub fn neural_pitch_implementation_hash() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("models.rs"));
    hash.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", hash.finalize())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchedSpan {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub midi: u8,
    pub clarity: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generation {
    pub version: u32,
    pub audio_sha256: String,
    pub vocal_stem_sha256: String,
    pub duration_seconds: f64,
    pub algorithm: String,
    /// Original tick/tempo/offset coordinates. Missing legacy metadata cannot
    /// establish that a cached tick-based chart belongs to a new tempo map.
    #[serde(default)]
    pub timing: Option<TempoMap>,
    pub policy: Policy,
    pub status: Status,
    /// Absent when no accepted pitch remains. Missing words use typed missing
    /// lyrics rather than deleting pitch or inventing a syllable.
    pub chart: Option<VocalChart>,
    pub pitch_candidates: Vec<PitchedSpan>,
    pub pitch_frames: Vec<PitchFrame>,
    pub words: Option<WordTimings>,
    pub lyrics: Option<LyricSource>,
    pub issues: Vec<Issue>,
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural_pitch: Option<NeuralPitchEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocal_pitch: Option<crate::vocal_pitch::Evidence>,
}
pub struct Request<'a> {
    pub audio: &'a Path,
    pub vocal_stem: &'a Path,
    pub alignment: &'a StemAlignment,
    pub timing: &'a TempoMap,
    pub words: Option<&'a WordTimings>,
    pub lyrics: Option<&'a LyricSource>,
    pub policy: &'a Policy,
    pub neural_pitch: Option<&'a NeuralPitchEvidence>,
    pub vocal_pitch: Option<&'a crate::vocal_pitch::Evidence>,
}

/// Audio must be an isolated, full-length, explicitly source-aligned vocal
/// estimate. Identical duration alone never establishes alignment.
pub fn generate(request: Request<'_>) -> Result<Generation> {
    generate_cancellable(request, &|| false)
}

/// Cancellation is checked between decode stages and throughout DSP. No
/// partial chart is published when analysis is cancelled.
pub fn generate_cancellable(
    request: Request<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Generation> {
    ensure!(!cancelled(), "Vocal generation cancelled");
    let audio_hash = models::hash_file(request.audio)?;
    let stem_hash = models::hash_file(request.vocal_stem)?;
    ensure!(
        request
            .alignment
            .audio_sha256
            .eq_ignore_ascii_case(&audio_hash)
            && request
                .alignment
                .stem_sha256
                .eq_ignore_ascii_case(&stem_hash)
            && !request.alignment.evidence.trim().is_empty(),
        "Vocal stem does not match the source-bound alignment evidence"
    );
    let original = models::decode(request.audio, 44100, 2)?;
    ensure!(!cancelled(), "Vocal generation cancelled");
    let stem = models::decode(request.vocal_stem, 44100, 2)?;
    ensure!(
        original.len() == stem.len(),
        "Vocal stem must span the original recording's complete sample clock"
    );
    let duration = original.len() as f64 / 88200.;
    drop(original);
    drop(stem);
    if let Some(evidence) = request.vocal_pitch {
        let pcm = models::decode(request.vocal_stem, 16000, 1)?;
        let mut hash = Sha256::new();
        for sample in pcm {
            hash.update(sample.to_le_bytes());
        }
        ensure!(
            format!("{:x}", hash.finalize()) == evidence.pcm_sha256,
            "Vocal pitch evidence does not match decoded source PCM"
        );
    }
    let signal = models::decode(request.vocal_stem, RATE as u32, 1)?;
    let generated = generate_pcm_cancellable(
        &signal,
        &audio_hash,
        &stem_hash,
        duration,
        request.timing,
        request.words,
        request.lyrics,
        request.policy,
        request.neural_pitch,
        request.vocal_pitch,
        cancelled,
    )?;
    ensure!(
        models::hash_file(request.audio)? == audio_hash
            && models::hash_file(request.vocal_stem)? == stem_hash,
        "Recording or vocal stem changed during generation; proposals were discarded"
    );
    Ok(generated)
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit())
}

pub(crate) fn validate_vocal_frame_evidence(
    e: &crate::vocal_pitch::Evidence,
    audio: &str,
    stem: &str,
    duration: f64,
) -> Result<()> {
    ensure!(
        e.version == 1
            && e.model_id == crate::vocal_pitch::MODEL_ID
            && e.model_revision == crate::vocal_pitch::REVISION
            && e.model_sha256 == crate::vocal_pitch::MODEL_SHA256
            && e.implementation_sha256 == crate::vocal_pitch::implementation_hash()
            && e.audio_sha256.eq_ignore_ascii_case(audio)
            && e.vocal_stem_sha256.eq_ignore_ascii_case(stem)
            && e.duration_seconds.is_finite()
            && (e.duration_seconds - duration).abs() <= 1. / 44100.
            && e.sample_rate == 16000
            && e.hop_samples == 256
            && digest(&e.pcm_sha256),
        "Vocal frame evidence belongs to another recording/stem/model/implementation/clock"
    );
    let nominal = e.frames.len() as f64 * 0.016;
    ensure!(
        !e.frames.is_empty()
            && e.frames.len() <= 900_001
            && duration - nominal <= 0.0160625
            && ((duration < 0.016 && e.frames.len() == 1) || nominal - duration <= 0.0000625),
        "Vocal frame evidence does not span the original sample clock"
    );
    ensure!(
        e.frames
            .iter()
            .enumerate()
            .all(|(i, f)| f.time_seconds.is_finite()
                && (f.time_seconds - i as f64 * 0.016).abs() < 1e-9
                && f.time_seconds <= duration
                && f.confidence.is_finite()
                && (0.0..=1.0).contains(&f.confidence)
                && f.rms.is_finite()
                && f.rms >= 0.
                && f.frequency_hz
                    .is_none_or(|hz| hz.is_finite() && (46.875..=2093.75).contains(&hz))),
        "Invalid, missing, nonfinite or unordered vocal pitch frames"
    );
    Ok(())
}

/// Normalized correlation at the proposed period. This recomputes acoustic
/// support directly rather than trusting the upstream YIN clarity transform.
fn period_support(signal: &[f32], center: usize, midi: f64) -> f64 {
    period_support_window(signal, center, midi, WINDOW)
}
fn period_support_window(signal: &[f32], center: usize, midi: f64, window: usize) -> f64 {
    let frequency = 440. * 2f64.powf((midi - 69.) / 12.);
    let lag = (RATE as f64 / frequency).round() as usize;
    if lag == 0 || lag >= window / 2 || center < window / 2 || center + window / 2 > signal.len() {
        return 0.;
    }
    let values = &signal[center - window / 2..center + window / 2];
    let mean = values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64;
    let (mut cross, mut a, mut b) = (0., 0., 0.);
    for index in 0..values.len() - lag {
        let first = values[index] as f64 - mean;
        let second = values[index + lag] as f64 - mean;
        cross += first * second;
        a += first * first;
        b += second * second;
    }
    (cross / (a * b).sqrt().max(f64::EPSILON)).clamp(-1., 1.)
}

fn centered_rms(signal: &[f32], center: usize, length: usize) -> f64 {
    let start = center.saturating_sub(length / 2).min(signal.len());
    let end = (center + length / 2).min(signal.len());
    if start >= end {
        return 0.;
    }
    let values = &signal[start..end];
    let mean = values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64;
    (values
        .iter()
        .map(|v| (*v as f64 - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

#[derive(Default, Serialize)]
struct SwiftStages {
    missing_consensus: usize,
    known_word: usize,
    local_energy: usize,
    model_confident: usize,
    dsp_compatible: usize,
    stable_model: usize,
    period_supported: usize,
    recovered: usize,
}
fn recover_swift_frames(
    frames: &mut [PitchFrame],
    signal: &[f32],
    words: Option<&WordTimings>,
    evidence: &crate::vocal_pitch::Evidence,
    gate: f64,
    issues: &mut Vec<Issue>,
) -> SwiftStages {
    let mut stages = SwiftStages::default();
    let Some(words) = words else {
        return stages;
    };
    if evidence.frames.is_empty() {
        return stages;
    }
    for frame in frames.iter_mut() {
        if frame.midi.is_some() || frame.local_energy_active == Some(false) {
            continue;
        }
        stages.missing_consensus += 1;
        if !words.words.iter().any(|w| {
            w.text.is_some()
                && w.start
                    .zip(w.end)
                    .is_some_and(|(s, e)| s <= frame.time_seconds && frame.time_seconds < e)
        }) {
            continue;
        }
        stages.known_word += 1;
        let center = (frame.time_seconds * RATE as f64).round() as usize;
        if centered_rms(signal, center, HOP) < gate || frame.rms < gate {
            continue;
        }
        stages.local_energy += 1;
        let at = evidence
            .frames
            .partition_point(|f| f.time_seconds < frame.time_seconds);
        let nearest = [at.saturating_sub(1), at.min(evidence.frames.len() - 1)]
            .into_iter()
            .min_by(|&a, &b| {
                (evidence.frames[a].time_seconds - frame.time_seconds)
                    .abs()
                    .total_cmp(&(evidence.frames[b].time_seconds - frame.time_seconds).abs())
            })
            .unwrap();
        let proposal = &evidence.frames[nearest];
        let Some(frequency) = proposal.frequency_hz else {
            continue;
        };
        let midi = 69. + 12. * (frequency / 440.).log2();
        if proposal.confidence < 0.8
            || (proposal.time_seconds - frame.time_seconds).abs() > 0.00800001
            || !(35.5..84.5).contains(&midi)
        {
            continue;
        }
        stages.model_confident += 1;
        if frame.yin_midi.is_some_and(|m| (m - midi).abs() > 0.55)
            || frame.mcleod_midi.is_some_and(|m| (m - midi).abs() > 0.55)
            || frame
                .yin_midi
                .zip(frame.mcleod_midi)
                .is_some_and(|(y, m)| (y - m).abs() > 0.55)
        {
            continue;
        }
        stages.dsp_compatible += 1;
        if evidence.frames[nearest.saturating_sub(2)..(nearest + 3).min(evidence.frames.len())]
            .iter()
            .filter(|f| {
                f.confidence >= 0.8
                    && f.frequency_hz
                        .is_some_and(|hz| (69. + 12. * (hz / 440.).log2() - midi).abs() <= 0.7)
            })
            .count()
            < 3
        {
            continue;
        }
        stages.stable_model += 1;
        let period = period_support(signal, center, midi);
        if period < 0.75 {
            continue;
        }
        stages.period_supported += 1;
        let shorter = [12., 19.01955000865, 24.]
            .into_iter()
            .map(|d| period_support(signal, center, midi + d))
            .fold(-1., f64::max);
        if period - shorter < 0.15 {
            continue;
        }
        frame.midi = Some(midi);
        frame.clarity = period.min(proposal.confidence as f64) * 0.5;
        frame.support = PitchSupport::SwiftF0;
        issue(
            issues,
            frame.time_seconds,
            frame.time_seconds + HOP as f64 / RATE as f64,
            Reason::PitchDisagreement,
            "SwiftF0 recovery: known word, source-frame energy, no raw DSP conflict, stable high-confidence model contour and direct fundamental-period support. Listening review remains required.",
        );
        stages.recovered += 1;
    }
    stages
}

/// Recover a bounded detector dropout, never an unanchored melody or an
/// octave disagreement. Both ends must have stable consensus; each recovered
/// frame additionally needs measured periodicity and known timed-word context.
fn recover_yin_gaps(
    frames: &mut [PitchFrame],
    signal: &[f32],
    words: Option<&WordTimings>,
    issues: &mut Vec<Issue>,
) {
    let Some(words) = words else {
        return;
    };
    let trusted: Vec<_> = frames.iter().map(|frame| frame.midi).collect();
    let stable_anchor = |index: usize, pitch: f64| {
        let nearby = &trusted[index.saturating_sub(4)..(index + 5).min(trusted.len())];
        nearby
            .iter()
            .filter_map(|v| *v)
            .filter(|v| (*v - pitch).abs() <= 0.6)
            .count()
            >= 3
    };
    let mut pending = Vec::new();
    let mut index = 0;
    while index < frames.len() {
        if trusted[index].is_some() {
            index += 1;
            continue;
        }
        let start = index;
        while index < frames.len() && trusted[index].is_none() {
            index += 1;
        }
        if start == 0 || index == frames.len() || index - start > 80 {
            continue;
        }
        let (Some(before), Some(after)) = (trusted[start - 1], trusted[index]) else {
            continue;
        };
        if (before - after).abs() > 7.
            || !stable_anchor(start - 1, before)
            || !stable_anchor(index, after)
        {
            continue;
        }
        for at in start..index {
            let frame = &frames[at];
            let Some(yin) = frame.yin_midi else {
                continue;
            };
            // An explicit conflicting estimate is not a detector dropout.
            if frame.mcleod_midi.is_some()
                || frame.local_energy_active == Some(false)
                || !(35.5..84.5).contains(&yin)
                || yin < before.min(after) - 2.
                || yin > before.max(after) + 2.
            {
                continue;
            }
            if !words.words.iter().any(|word| {
                word.text.is_some()
                    && word
                        .start
                        .zip(word.end)
                        .is_some_and(|(a, b)| a <= frame.time_seconds && frame.time_seconds < b)
            }) {
                continue;
            }
            let mut nearby: Vec<_> = frames[at.saturating_sub(2)..(at + 3).min(frames.len())]
                .iter()
                .filter_map(|f| f.yin_midi)
                .collect();
            if nearby.len() < 4 {
                continue;
            }
            nearby.sort_by(f64::total_cmp);
            if nearby.last().unwrap() - nearby.first().unwrap() > 0.70
                || (nearby[nearby.len() / 2] - yin).abs() > 0.4
            {
                continue;
            }
            let acoustic = period_support(
                signal,
                (frame.time_seconds * RATE as f64).round() as usize,
                yin,
            );
            if acoustic < 0.85 {
                continue;
            }
            pending.push((at, yin, acoustic));
        }
    }
    for (at, pitch, acoustic) in pending {
        let frame = &mut frames[at];
        frame.midi = Some(pitch);
        frame.clarity = acoustic * 0.5;
        frame.support = PitchSupport::AnchoredYin;
        issue(
            issues,
            frame.time_seconds,
            frame.time_seconds + HOP as f64 / RATE as f64,
            Reason::PitchDisagreement,
            "Recovered a brief YIN-only contour using stable dual-detector anchors, measured period correlation and timed-word context. McLeod did not provide a pitch; listening review remains required.",
        );
    }
}

/// An independent neural vote may support a contour without nearby McLeod
/// anchors. Never choose between conflicting monophonic detector estimates.
fn recover_neural_yin(
    frames: &mut [PitchFrame],
    signal: &[f32],
    words: Option<&WordTimings>,
    neural: &NeuralPitchEvidence,
    issues: &mut Vec<Issue>,
) {
    let Some(words) = words else {
        return;
    };
    let mut pending = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let Some(yin) = frame.yin_midi else {
            continue;
        };
        if frame.midi.is_some()
            || frame.local_energy_active == Some(false)
            || frame.mcleod_midi.is_some()
            || !(35.5..84.5).contains(&yin)
            || !words.words.iter().any(|w| {
                w.text.is_some()
                    && w.start
                        .zip(w.end)
                        .is_some_and(|(a, b)| a <= frame.time_seconds && frame.time_seconds < b)
            })
        {
            continue;
        }
        let active: Vec<_> = neural
            .notes
            .iter()
            .filter(|note| note.start <= frame.time_seconds && frame.time_seconds < note.end)
            .collect();
        let Some(note) = active
            .iter()
            .copied()
            .filter(|note| note.confidence >= 0.6 && (note.pitch as f64 - yin).abs() <= 0.5)
            .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
        else {
            continue;
        };
        // Higher integer harmonics are expected in a polyphonic network. A
        // non-harmonic alternative (including a lower octave) must be weaker.
        let competing = active
            .iter()
            .filter(|other| other.pitch != note.pitch)
            .filter(|other| {
                ![12, 19, 24, 28, 31].contains(&(other.pitch as i16 - note.pitch as i16))
            })
            .map(|other| other.confidence)
            .fold(0., f32::max);
        if note.confidence - competing < 0.15 {
            continue;
        }
        let mut nearby: Vec<_> = frames[index.saturating_sub(2)..(index + 3).min(frames.len())]
            .iter()
            .filter_map(|f| f.yin_midi)
            .collect();
        if nearby.len() < 4 {
            continue;
        }
        nearby.sort_by(f64::total_cmp);
        if nearby.last().unwrap() - nearby.first().unwrap() > 0.7
            || (nearby[nearby.len() / 2] - yin).abs() > 0.4
        {
            continue;
        }
        let center = (frame.time_seconds * RATE as f64).round() as usize;
        let correlation = period_support(signal, center, yin);
        if correlation < 0.75 {
            continue;
        }
        // A pure higher harmonic also repeats at the proposed low period.
        // Requiring stronger full-period evidence prevents that octave error.
        let shorter = [12., 19.01955000865, 24.]
            .into_iter()
            .map(|delta| period_support(signal, center, yin + delta))
            .fold(-1., f64::max);
        if correlation - shorter < 0.15 {
            continue;
        }
        pending.push((index, yin, correlation.min(note.confidence as f64) * 0.5));
    }
    for (index, yin, clarity) in pending {
        let frame = &mut frames[index];
        frame.midi = Some(yin);
        frame.clarity = clarity;
        frame.support = PitchSupport::NeuralYin;
        issue(
            issues,
            frame.time_seconds,
            frame.time_seconds + HOP as f64 / RATE as f64,
            Reason::PitchDisagreement,
            "Recovered a YIN contour with an independent local Basic Pitch vote, non-harmonic score margin and measured fundamental-period contrast. McLeod did not provide a pitch; model agreement is not musical approval.",
        );
    }
}
fn issue(issues: &mut Vec<Issue>, start: f64, end: f64, reason: Reason, detail: &str) {
    // Group nearby frame warnings into sections the reviewer can jump to.
    if let Some(previous) = issues.last_mut()
        && previous.reason == reason
        && start >= previous.start_seconds
        && start <= previous.end_seconds + 0.025
    {
        previous.end_seconds = previous.end_seconds.max(end);
        return;
    }
    issues.push(Issue {
        start_seconds: start,
        end_seconds: end,
        reason,
        detail: detail.into(),
    });
}

fn bound_issue_ranges(issues: &mut Vec<Issue>, duration: f64) {
    // A final detector hop can extend beyond the last source sample. Keep
    // diagnostics on that source clock; this does not change pitch evidence.
    for issue in issues.iter_mut() {
        issue.start_seconds = issue.start_seconds.clamp(0., duration);
        issue.end_seconds = issue.end_seconds.clamp(issue.start_seconds, duration);
    }
    issues.retain(|issue| issue.start_seconds < duration);
}

/// Model-free core. `signal` is mono 22050 Hz; duration retains the original
/// 44100 Hz clock (resampling can round its last sample). No models/network.
#[allow(clippy::too_many_arguments)]
pub fn generate_pcm(
    signal: &[f32],
    audio_sha256: &str,
    stem_sha256: &str,
    duration: f64,
    timing: &TempoMap,
    words: Option<&WordTimings>,
    lyrics: Option<&LyricSource>,
    policy: &Policy,
) -> Result<Generation> {
    generate_pcm_cancellable(
        signal,
        audio_sha256,
        stem_sha256,
        duration,
        timing,
        words,
        lyrics,
        policy,
        None,
        None,
        &|| false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn generate_pcm_with_neural_pitch(
    signal: &[f32],
    audio_sha256: &str,
    stem_sha256: &str,
    duration: f64,
    timing: &TempoMap,
    words: Option<&WordTimings>,
    lyrics: Option<&LyricSource>,
    policy: &Policy,
    neural: &NeuralPitchEvidence,
) -> Result<Generation> {
    generate_pcm_cancellable(
        signal,
        audio_sha256,
        stem_sha256,
        duration,
        timing,
        words,
        lyrics,
        policy,
        Some(neural),
        None,
        &|| false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn generate_pcm_with_vocal_pitch(
    signal: &[f32],
    audio_sha256: &str,
    stem_sha256: &str,
    duration: f64,
    timing: &TempoMap,
    words: Option<&WordTimings>,
    lyrics: Option<&LyricSource>,
    policy: &Policy,
    neural: Option<&NeuralPitchEvidence>,
    vocal_pitch: &crate::vocal_pitch::Evidence,
) -> Result<Generation> {
    generate_pcm_cancellable(
        signal,
        audio_sha256,
        stem_sha256,
        duration,
        timing,
        words,
        lyrics,
        policy,
        neural,
        Some(vocal_pitch),
        &|| false,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_pcm_cancellable(
    signal: &[f32],
    audio_sha256: &str,
    stem_sha256: &str,
    duration: f64,
    timing: &TempoMap,
    words: Option<&WordTimings>,
    lyrics: Option<&LyricSource>,
    policy: &Policy,
    neural: Option<&NeuralPitchEvidence>,
    vocal_pitch: Option<&crate::vocal_pitch::Evidence>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Generation> {
    ensure!(!cancelled(), "Vocal generation cancelled");
    policy.validate()?;
    ensure!(
        digest(audio_sha256) && digest(stem_sha256),
        "Vocal generation needs source and stem SHA-256 identities"
    );
    ensure!(
        duration.is_finite() && duration > 0. && duration <= 14400.,
        "Invalid vocal source duration"
    );
    ensure!(
        (signal.len() as f64 / RATE as f64 - duration).abs() <= 2. / RATE as f64,
        "Vocal PCM duration does not match the original sample clock"
    );
    ensure!(
        signal.iter().all(|v| v.is_finite()),
        "Vocal PCM contains nonfinite samples"
    );
    if let Some(neural) = neural {
        let asset = models::assets()
            .into_iter()
            .find(|asset| asset.id == "pitch")
            .unwrap();
        ensure!(
            neural.audio_sha256.eq_ignore_ascii_case(audio_sha256)
                && neural.vocal_stem_sha256.eq_ignore_ascii_case(stem_sha256)
                && (neural.duration_seconds - duration).abs() <= 1. / 44100.
                && neural.model_sha256 == asset.sha256
                && neural.implementation_sha256 == neural_pitch_implementation_hash(),
            "Neural vocal pitch evidence belongs to another recording/stem/model/implementation"
        );
        ensure!(
            neural.notes.len() <= 200_000
                && neural.notes.iter().all(|note| note.start.is_finite()
                    && note.end.is_finite()
                    && note.start >= 0.
                    && note.start < note.end
                    && note.end <= duration
                    && note.pitch > 0
                    && note.pitch <= 127
                    && note.confidence.is_finite()
                    && (0.0..=1.0).contains(&note.confidence)),
            "Invalid neural vocal note evidence"
        );
    }
    if let Some(evidence) = vocal_pitch {
        validate_vocal_frame_evidence(evidence, audio_sha256, stem_sha256, duration)?;
    }
    // Validate caller-constructed timing maps, not just deserialized maps.
    let timing = TempoMap::new(timing.resolution, timing.offset, timing.tempos.clone())?;
    ensure!(
        timing.resolution <= 32767,
        "Vocal MIDI PPQ exceeds the supported range"
    );
    if let Some(words) = words {
        words.validate()?;
        ensure!(
            words.audio_sha256.eq_ignore_ascii_case(audio_sha256)
                && (words.duration_seconds - duration).abs() <= 1. / 44100.,
            "Timed lyrics belong to another recording/sample clock"
        );
    }
    if let Some(lyrics) = lyrics {
        ensure!(
            !lyrics.source.trim().is_empty(),
            "Lyrics require provenance"
        );
    }
    let mut issues = Vec::new();
    let mut yin = YINDetector::<f64>::new(WINDOW, WINDOW / 2);
    let mut mcleod = McLeodDetector::<f64>::new(WINDOW, WINDOW / 2);
    // RMS is measured per frame; relative gate is invariant to the stem gain.
    let peak_rms = signal
        .chunks(HOP)
        .map(|chunk| {
            (chunk.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / chunk.len() as f64).sqrt()
        })
        .fold(0., f64::max);
    let gate = policy
        .absolute_rms_floor
        .max(peak_rms * policy.relative_rms_floor);
    let mut frames = Vec::new();
    for center in (0..signal.len()).step_by(HOP) {
        ensure!(!cancelled(), "Vocal generation cancelled");
        let time = center as f64 / RATE as f64;
        let mut buffer = vec![0.; WINDOW];
        for (index, sample) in buffer.iter_mut().enumerate() {
            let source = center as isize + index as isize - WINDOW as isize / 2;
            if source >= 0
                && let Some(value) = signal.get(source as usize)
            {
                *sample = *value as f64;
            }
        }
        let mean = buffer.iter().sum::<f64>() / WINDOW as f64;
        for sample in &mut buffer {
            *sample -= mean;
        }
        let rms = (buffer.iter().map(|v| v * v).sum::<f64>() / WINDOW as f64).sqrt();
        let local_rms = centered_rms(signal, center, HOP);
        let mut frame = PitchFrame {
            time_seconds: time,
            midi: None,
            yin_midi: None,
            mcleod_midi: None,
            clarity: 0.,
            rms,
            local_rms: Some(local_rms),
            local_energy_active: policy.local_energy_gate.then_some(local_rms >= gate),
            support: PitchSupport::Unspecified,
        };
        if rms >= gate {
            let y = yin.get_pitch(&buffer, RATE, 0.00001, 0.80);
            let m = mcleod.get_pitch(&buffer, RATE, 0.00001, 0.85);
            frame.yin_midi = y
                .as_ref()
                .map(|p| 69. + 12. * (p.frequency / 440.).log2())
                .filter(|v| v.is_finite());
            frame.mcleod_midi = m
                .as_ref()
                .map(|p| 69. + 12. * (p.frequency / 440.).log2())
                .filter(|v| v.is_finite());
            if let (Some(y), Some(m)) = (y, m) {
                let a = 69. + 12. * (y.frequency / 440.).log2();
                let b = 69. + 12. * (m.frequency / 440.).log2();
                if a.is_finite() && b.is_finite() && (a - b).abs() <= 0.55 {
                    if (35.5..84.5).contains(&a) {
                        if frame.local_energy_active != Some(false) {
                            frame.midi = Some((a + b) * 0.5);
                            frame.support = PitchSupport::Consensus;
                        }
                        // pitch-detection 0.3's YIN clarity transform can be
                        // negative for a good period. Use measured frequency
                        // agreement and McLeod NSDF clarity, never that value.
                        frame.clarity = (1. - (a - b).abs() / 0.55).min(m.clarity).clamp(0., 1.);
                    } else {
                        issue(
                            &mut issues,
                            time,
                            (time + HOP as f64 / RATE as f64).min(duration),
                            Reason::OutOfRangePitch,
                            "Pitch is outside YARG's MIDI 36..84 vocal range; it was not transposed.",
                        );
                    }
                } else {
                    issue(
                        &mut issues,
                        time,
                        (time + HOP as f64 / RATE as f64).min(duration),
                        Reason::PitchDisagreement,
                        "The pitch detectors disagree; backing vocals, breath or separation leakage may be present.",
                    );
                }
            }
        }
        frames.push(frame);
    }
    let aligned = if let (Some(words), Some(lyrics)) = (words, lyrics) {
        let (aligned, reconciliation_issues) = reconcile_lyric_evidence(words, lyrics)?;
        issues.extend(reconciliation_issues);
        Some(aligned)
    } else {
        words.cloned()
    };
    if policy.recover_bounded_yin {
        recover_yin_gaps(&mut frames, signal, aligned.as_ref(), &mut issues);
    }
    if let Some(neural) = neural {
        recover_neural_yin(&mut frames, signal, aligned.as_ref(), neural, &mut issues);
    }
    if let Some(evidence) = vocal_pitch {
        recover_swift_frames(
            &mut frames,
            signal,
            aligned.as_ref(),
            evidence,
            gate,
            &mut issues,
        );
    }
    let spans = pitch_spans(&frames, duration, policy.minimum_note_seconds);
    let chart = chart_from_spans(
        &spans,
        &frames,
        aligned.as_ref(),
        &timing,
        policy,
        &mut issues,
    )?;
    let status = if spans.is_empty() {
        Status::NoVoicedPitch
    } else if chart.as_ref().is_none_or(|chart| {
        chart
            .notes
            .iter()
            .all(|note| note.lyric == Syllable::Missing)
    }) {
        Status::NeedsLyrics
    } else {
        Status::ReviewRequired
    };
    let mut warnings = vec!["Automatic vocal pitch, word boundaries and breathing phrases are proposals. Unison/backing vocals, consonants and stem leakage need listening review; detector agreement is not musical acceptance.".into()];
    if chart.as_ref().is_some_and(|chart| {
        chart
            .notes
            .iter()
            .any(|note| note.lyric == Syllable::Missing)
    }) {
        warnings.push("Accepted pitch without a timed word is retained as a pitched note with a missing lyric. Add words in the vocal note inspector; missing words do not delete pitches or become talkies.".into());
    }
    if lyrics.is_some() && aligned.is_none() {
        warnings.push("Fetched plain/line-synced lyrics are retained as text; line timestamps are not word timing.".into());
    }
    bound_issue_ranges(&mut issues, duration);
    issues.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    Ok(Generation {
        version: 1,
        audio_sha256: audio_sha256.into(),
        vocal_stem_sha256: stem_sha256.into(),
        duration_seconds: duration,
        algorithm: ALGORITHM.into(),
        timing: Some(timing.clone()),
        policy: policy.clone(),
        status,
        chart,
        pitch_candidates: spans,
        pitch_frames: frames,
        words: aligned,
        lyrics: lyrics.cloned(),
        issues,
        warnings,
        neural_pitch: neural.cloned(),
        vocal_pitch: vocal_pitch.cloned(),
    })
}

fn pitch_spans(
    frames: &[PitchFrame],
    duration: f64,
    minimum_note_seconds: f64,
) -> Vec<PitchedSpan> {
    // Smooth voiced frames to reduce vibrato and isolated pitch jumps.
    // Leave breaths and unvoiced frames empty.
    let medians: Vec<_> = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            frame.midi.map(|_| {
                let mut pitches: Vec<_> = frames
                    [index.saturating_sub(2)..(index + 3).min(frames.len())]
                    .iter()
                    .filter_map(|f| f.midi)
                    .collect();
                pitches.sort_by(f64::total_cmp);
                pitches[pitches.len() / 2]
            })
        })
        .collect();
    // Ordinary vibrato must not become rapid neighbouring notes. A real
    // change must leave the previous semitone's +/-70-cent region.
    let mut last = None;
    let mut smoothed = Vec::with_capacity(medians.len());
    for value in medians {
        let pitch = value.map(|value| match last {
            Some(previous) if (value - previous as f64).abs() < 0.70 => previous,
            _ => value.round() as u8,
        });
        last = pitch;
        smoothed.push(pitch);
    }
    let mut spans: Vec<PitchedSpan> = Vec::new();
    let hop = HOP as f64 / RATE as f64;
    for (index, frame) in frames.iter().enumerate() {
        let Some(midi) = smoothed[index] else {
            continue;
        };
        let start = (frame.time_seconds - hop / 2.).max(0.);
        let end = (frame.time_seconds + hop / 2.).min(duration);
        if let Some(previous) = spans.last_mut()
            && previous.midi == midi
            && start <= previous.end_seconds + 0.025
        {
            previous.end_seconds = end;
            previous.clarity = previous.clarity.min(frame.clarity);
            continue;
        }
        spans.push(PitchedSpan {
            start_seconds: start,
            end_seconds: end,
            midi,
            clarity: frame.clarity,
        });
    }
    spans.retain(|n| n.end_seconds - n.start_seconds >= minimum_note_seconds);
    spans
}

/// Match provider lyrics to recognized words in order.
/// Keep unmatched recognized words at their measured times. Leave unmatched
/// provider words untimed rather than borrowing another word's timestamp.
pub fn reconcile_lyrics(recognized: &WordTimings, lyrics: &LyricSource) -> Result<WordTimings> {
    reconcile_lyric_evidence(recognized, lyrics).map(|(words, _)| words)
}

fn reconcile_lyric_evidence(
    recognized: &WordTimings,
    lyrics: &LyricSource,
) -> Result<(WordTimings, Vec<Issue>)> {
    recognized.validate()?;
    ensure!(
        !lyrics.source.trim().is_empty(),
        "Lyrics require provenance"
    );
    let supplied = crate::clean::tokens(&lyrics.text);
    let heard: Vec<_> = recognized
        .words
        .iter()
        .map(|w| {
            w.text
                .as_deref()
                .map(crate::clean::tokens)
                .and_then(|mut t| (t.len() == 1).then(|| t.remove(0)))
        })
        .collect();
    ensure!(
        supplied.len().saturating_mul(heard.len()) <= 4_000_000,
        "Lyrics reconciliation exceeds its bounded word budget"
    );
    let width = heard.len() + 1;
    let mut lengths = vec![0u32; (supplied.len() + 1) * width];
    for i in (0..supplied.len()).rev() {
        for j in (0..heard.len()).rev() {
            lengths[i * width + j] = if heard[j].as_deref() == Some(&supplied[i]) {
                1 + lengths[(i + 1) * width + j + 1]
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let mut words = Vec::with_capacity(supplied.len() + heard.len());
    let mut issues = Vec::new();
    let retain_recognized = |index: usize,
                             words: &mut Vec<AlignedWord>,
                             issues: &mut Vec<Issue>| {
        let word = recognized.words[index].clone();
        if let Some((start, end)) = word.start.zip(word.end) {
            issues.push(Issue {
                start_seconds: start,
                end_seconds: end,
                reason: Reason::LyricDisagreement,
                detail: "A locally recognized word has measured timing but no ordered match in the supplied lyrics. Its original text and timing were retained as an uncertain draft; compare the recording and lyric sources.".into(),
            });
        }
        words.push(word);
    };
    let (mut i, mut j) = (0, 0);
    while i < supplied.len() && j < heard.len() {
        if heard[j].as_deref() == Some(&supplied[i]) {
            words.push(AlignedWord {
                text: Some(supplied[i].clone()),
                ..recognized.words[j].clone()
            });
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            words.push(AlignedWord {
                text: Some(supplied[i].clone()),
                start: None,
                end: None,
            });
            i += 1;
        } else {
            retain_recognized(j, &mut words, &mut issues);
            j += 1;
        }
    }
    words.extend(supplied[i..].iter().map(|text| AlignedWord {
        text: Some(text.clone()),
        start: None,
        end: None,
    }));
    for index in j..heard.len() {
        retain_recognized(index, &mut words, &mut issues);
    }
    let result = WordTimings {
        words,
        source: format!(
            "{}; recognition-preserving ordered lexical reconciliation against {}; provider-only words remain untimed; recognition-only words retain their measured bounds and require source comparison",
            recognized.source, lyrics.source
        ),
        ..recognized.clone()
    };
    result.validate()?;
    Ok((result, issues))
}

fn chart_from_spans(
    spans: &[PitchedSpan],
    frames: &[PitchFrame],
    words: Option<&WordTimings>,
    timing: &TempoMap,
    policy: &Policy,
    issues: &mut Vec<Issue>,
) -> Result<Option<VocalChart>> {
    let mut proposals: Vec<(VocalNote, Option<usize>, f64, f64)> = Vec::new();
    if let Some(words) = words {
        for (word_index, word) in words.words.iter().enumerate() {
            let (Some(start), Some(end), Some(text)) = (word.start, word.end, word.text.as_ref())
            else {
                let before = words.words[..word_index]
                    .iter()
                    .rev()
                    .find_map(|word| word.end)
                    .unwrap_or(0.);
                let after = words.words[word_index + 1..]
                    .iter()
                    .find_map(|word| word.start)
                    .unwrap_or(words.duration_seconds);
                issue(
                    issues,
                    before.min(after),
                    after.max(before),
                    Reason::UntimedWord,
                    "Some supplied/recognized words have unknown timing and were not spread evenly across a line.",
                );
                continue;
            };
            let lexical = crate::clean::tokens(text);
            let Some(text) = lexical.first() else {
                continue;
            };
            let mut matched = false;
            for span in spans {
                let a = start.max(span.start_seconds);
                let b = end.min(span.end_seconds);
                if b - a < 0.030 {
                    continue;
                }
                // Overlapping source words have ambiguous ownership. Do not
                // produce overlapping notes or arbitrarily pick a word.
                if words.words.iter().enumerate().any(|(index, other)| {
                    index != word_index
                        && other
                            .start
                            .zip(other.end)
                            .is_some_and(|(s, e)| s < b && e > a)
                }) {
                    continue;
                }
                let (Some(tick), Some(tail)) = (timing.tick(a), timing.tick(b)) else {
                    continue;
                };
                if tail <= tick {
                    continue;
                }
                if b - a < policy.minimum_note_seconds {
                    issue(
                        issues,
                        a,
                        b,
                        Reason::ShortWordFragment,
                        "A stable pitch contour was clipped to a short word-boundary intersection. Original timing and pitch were retained; inspect the word/syllable boundary and articulation rather than stretching or dropping the note.",
                    );
                }
                matched = true;
                proposals.push((
                    VocalNote {
                        start: tick,
                        end: tail,
                        tone: VocalTone::Pitched { midi: span.midi },
                        lyric: Syllable::Text {
                            text: text.clone(),
                            join_next: false,
                        },
                    },
                    Some(word_index),
                    a,
                    b,
                ));
            }
            if matched {
                issue(
                    issues,
                    start,
                    end,
                    Reason::WordLevelAlignment,
                    "Word timing is available; syllable boundaries and melisma interpretation still require review.",
                );
            } else {
                issue(
                    issues,
                    start,
                    end,
                    Reason::WordWithoutPitch,
                    "This timed word has no stable in-range voiced pitch. No talkie or arbitrary sung note was substituted.",
                );
            }
        }
    }
    for span in spans {
        let mut covered: Vec<_> = proposals
            .iter()
            .filter_map(|p| {
                let start = p.2.max(span.start_seconds);
                let end = p.3.min(span.end_seconds);
                (start < end).then_some((start, end))
            })
            .collect();
        covered.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cursor = span.start_seconds;
        for (start, end) in covered
            .into_iter()
            .chain(std::iter::once((span.end_seconds, span.end_seconds)))
        {
            if start > cursor {
                issue(
                    issues,
                    cursor,
                    start,
                    Reason::MissingLyrics,
                    "Accepted pitch has no unambiguous timed word. The pitched note is retained with a missing lyric; no text, word owner or connecting slide was invented.",
                );
                if let (Some(tick), Some(tail)) = (timing.tick(cursor), timing.tick(start))
                    && tick < tail
                {
                    proposals.push((
                        VocalNote {
                            start: tick,
                            end: tail,
                            tone: VocalTone::Pitched { midi: span.midi },
                            lyric: Syllable::Missing,
                        },
                        None,
                        cursor,
                        start,
                    ));
                }
            }
            cursor = cursor.max(end);
        }
    }
    if proposals.is_empty() {
        return Ok(None);
    }
    proposals.sort_by_key(|p| p.0.start);
    let mut chart = VocalChart {
        version: 1,
        resolution: timing.resolution,
        source: format!(
            "Automatic source-bound isolated vocal proposal; {ALGORITHM}; pitch-first with optional word alignment, review required"
        ),
        phrases: vec![],
        notes: vec![],
    };
    let mut phrase_start = proposals[0].0.start;
    let mut phrase_start_seconds = proposals[0].2;
    let mut last_word = None;
    let mut previous_seconds = proposals[0].2;
    for (mut note, word_index, start, end) in proposals {
        if let Some(previous) = chart.notes.last() {
            ensure!(
                previous.end <= note.start,
                "Vocal lyric proposals overlap after tempo conversion"
            );
            let gap_break = start - previous_seconds >= policy.phrase_gap_seconds;
            let long = start - phrase_start_seconds >= policy.maximum_phrase_seconds;
            if gap_break || long {
                if frames.iter().any(|frame| {
                    frame.time_seconds > previous_seconds
                        && frame.time_seconds < start
                        && (frame.midi.is_some() || frame.local_energy_active == Some(true))
                }) {
                    issue(
                        issues,
                        previous_seconds,
                        start,
                        Reason::CoveragePhraseBoundary,
                        "This scoring phrase splits at a gap in charted words, but the isolated stem has active energy or accepted pitch between the notes. Stem energy alone proves neither singing nor a breath; review lyric/pitch coverage and musical phrasing. No notes were filled or phrases merged.",
                    );
                }
                chart.phrases.push(VocalPhrase {
                    start: phrase_start,
                    end: previous.end,
                    player: 1,
                });
                phrase_start = note.start;
                phrase_start_seconds = start;
                last_word = None;
                if long && !gap_break {
                    issue(
                        issues,
                        start,
                        end,
                        Reason::PhraseBoundary,
                        "A long vocal run was split at a note boundary; check breathing/phrase placement.",
                    );
                }
            }
        }
        // Keep one word owner when a consonant or rejected frame splits its pitch.
        // The phrase check above stops the word continuing into another phrase.
        if word_index.is_some() && last_word == word_index {
            let previous = chart
                .notes
                .last()
                .expect("A continued word has a preceding note");
            if note.start.saturating_sub(previous.end) <= 1 {
                note.lyric = Syllable::Continue;
            } else {
                note.lyric = Syllable::ContinueWord;
                issue(
                    issues,
                    previous_seconds,
                    start,
                    Reason::UnsupportedPitchGap,
                    "Two measured contours belong to one known word, but their intervening pitch is unsupported. Continue the displayed word without inferring a connecting pitch slide or a new attack; review the gap against the isolated vocal.",
                );
            }
        }
        previous_seconds = end;
        last_word = word_index;
        chart.notes.push(note);
    }
    chart.phrases.push(VocalPhrase {
        start: phrase_start,
        end: chart.notes.last().unwrap().end,
        player: 1,
    });
    chart.validate()?;
    Ok(Some(chart))
}

#[cfg(test)]
mod coverage_probes {
    use super::*;
    /// Representation-only audit. Keeps the original analysis identity and
    /// never presents a cached detector/model result as newly inferred audio.
    #[test]
    #[ignore = "requires explicit YACE_VOCAL_ASSEMBLY_REPORT and YACE_VOCAL_ASSEMBLY_OUTPUT"]
    fn audit_cached_vocal_assembly_without_reanalysis() {
        use std::{fs, io::Write, path::PathBuf};
        let input = PathBuf::from(std::env::var("YACE_VOCAL_ASSEMBLY_REPORT").unwrap());
        let output = PathBuf::from(std::env::var("YACE_VOCAL_ASSEMBLY_OUTPUT").unwrap());
        let bytes = fs::read(&input).unwrap();
        let mut report: Generation = serde_json::from_slice(&bytes).unwrap();
        let timing = report.timing.as_ref().unwrap();
        let original = report.chart.as_ref().unwrap();
        let mut issues = Vec::new();
        let chart = chart_from_spans(
            &report.pitch_candidates,
            &report.pitch_frames,
            report.words.as_ref(),
            timing,
            &report.policy,
            &mut issues,
        )
        .unwrap()
        .unwrap();
        for old in &original.notes {
            assert!(
                chart.notes.iter().any(|note| note == old),
                "Every previous labeled note must survive unchanged"
            );
        }
        for tick in 0..timing.tick(report.duration_seconds).unwrap() {
            let expected = report
                .pitch_candidates
                .iter()
                .find(|span| {
                    timing.tick(span.start_seconds).unwrap() <= tick
                        && tick < timing.tick(span.end_seconds).unwrap()
                })
                .map(|span| VocalTone::Pitched { midi: span.midi });
            let actual = chart
                .notes
                .iter()
                .find(|note| note.start <= tick && tick < note.end)
                .map(|note| note.tone.clone());
            assert_eq!(actual, expected, "Cached pitch coverage at tick {tick}");
        }
        let base = crate::midi::new_midi(timing, &crate::chart::Tracks::new()).unwrap();
        let midi = crate::vocals::append_midi(&base, &chart).unwrap();
        let roundtrip = crate::vocals::import_midi(&midi).unwrap();
        assert_eq!(roundtrip.notes, chart.notes);
        assert_eq!(roundtrip.phrases, chart.phrases);
        let receipt = serde_json::json!({
            "condition": "representation-only assembly from unchanged cached analysis; no audio/model re-inference",
            "source_report": input, "source_report_sha256": format!("{:x}", Sha256::digest(&bytes)),
            "source_algorithm": report.algorithm, "assembly_algorithm": ALGORITHM,
            "audio_sha256": report.audio_sha256, "vocal_stem_sha256": report.vocal_stem_sha256,
            "timing": timing, "notes": chart.notes.len(), "phrases": chart.phrases.len(),
            "previous_labeled_notes_exactly_conserved": true,
            "accepted_pitch_coverage_exactly_conserved_in_ticks": true,
            "missing_lyric_notes": chart.notes.iter().filter(|n| n.lyric == Syllable::Missing).count(),
            "note_seconds": chart.notes.iter().map(|n| timing.seconds(n.end) - timing.seconds(n.start)).sum::<f64>(),
            "intentional_contiguous_slides": chart.notes.iter().filter(|n| n.lyric == Syllable::Continue).count(),
            "new_assembly_issues": issues,
            "midi_sha256": format!("{:x}", Sha256::digest(&midi)),
        });
        fs::create_dir_all(&output).unwrap();
        // Keep the original detector/model identity; only the chart assembly
        // changed. Stale model evidence is never relabeled as fresh inference.
        report.chart = Some(chart.clone());
        report.issues.retain(|issue| {
            matches!(
                issue.reason,
                Reason::PitchDisagreement | Reason::OutOfRangePitch | Reason::LyricDisagreement
            )
        });
        report.issues.extend(issues);
        report.warnings.push(format!("Chart assembly replayed using {ALGORITHM}; pitch frames, candidates, word timings and model evidence are unchanged cached analysis from {}. No new pitch inference or musical acceptance.", report.algorithm));
        for (name, data) in [
            ("receipt.json", serde_json::to_vec_pretty(&receipt).unwrap()),
            (
                "generation.json",
                serde_json::to_vec_pretty(&report).unwrap(),
            ),
            ("vocals.json", serde_json::to_vec_pretty(&chart).unwrap()),
            ("vocals.mid", midi),
        ] {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join(name))
                .unwrap();
            file.write_all(&data).unwrap();
        }
    }
    fn contour(start: f64, end: f64, midi: u8) -> PitchedSpan {
        PitchedSpan {
            start_seconds: start,
            end_seconds: end,
            midi,
            clarity: 0.99,
        }
    }
    fn assemble(
        spans: &[PitchedSpan],
        frames: &[PitchFrame],
        words: &WordTimings,
    ) -> (VocalChart, Vec<Issue>) {
        let mut issues = Vec::new();
        let chart = chart_from_spans(
            spans,
            frames,
            Some(words),
            &TempoMap::constant(120.).unwrap(),
            &Policy::default(),
            &mut issues,
        )
        .unwrap()
        .unwrap();
        (chart, issues)
    }
    #[test]
    fn same_word_slide_requires_contiguous_contours_and_conserves_measured_notes() {
        let words = words();
        for (second_start, expected) in [
            (0.8, Syllable::Continue),
            (0.8 + 1. / 960., Syllable::Continue),
            (1.0, Syllable::ContinueWord),
        ] {
            let spans = [contour(0.2, 0.8, 57), contour(second_start, 1.6, 62)];
            let (chart, issues) = assemble(&spans, &[], &words);
            let timing = TempoMap::constant(120.).unwrap();
            assert_eq!(chart.notes.len(), 2);
            for (note, span) in chart.notes.iter().zip(&spans) {
                assert_eq!(
                    (note.start, note.end),
                    (
                        timing.tick(span.start_seconds).unwrap(),
                        timing.tick(span.end_seconds).unwrap()
                    )
                );
                assert_eq!(note.tone, VocalTone::Pitched { midi: span.midi });
            }
            assert_eq!(chart.notes[1].lyric, expected);
            assert_eq!(
                issues
                    .iter()
                    .any(|i| i.reason == Reason::UnsupportedPitchGap),
                expected == Syllable::ContinueWord
            );
            assert_eq!(chart.phrases.len(), 1);
        }
    }
    #[test]
    fn missing_short_and_ambiguous_words_never_remove_measured_pitch() {
        let spans = [
            contour(0.1, 0.6, 57),
            contour(0.7, 1.1, 60),
            contour(1.2, 1.8, 59),
        ];
        let timing = TempoMap::constant(120.).unwrap();
        let mut timings = words();
        timings.words = vec![
            crate::clean::AlignedWord {
                text: Some("stay".into()),
                start: Some(0.2),
                end: Some(0.4),
            },
            // Two overlapping observations cannot establish lyric ownership.
            crate::clean::AlignedWord {
                text: Some("one".into()),
                start: Some(0.7),
                end: Some(1.0),
            },
            crate::clean::AlignedWord {
                text: Some("two".into()),
                start: Some(0.8),
                end: Some(1.1),
            },
            crate::clean::AlignedWord {
                text: Some("short".into()),
                start: Some(1.25),
                end: Some(1.27),
            },
        ];
        for words in [None, Some(&timings)] {
            let mut issues = Vec::new();
            let chart =
                chart_from_spans(&spans, &[], words, &timing, &Policy::default(), &mut issues)
                    .unwrap()
                    .unwrap();
            chart.validate().unwrap();
            // Exact tick-by-tick union of measured pitch, with no extension,
            // interpolation, or pitch substitution in uncovered word regions.
            for tick in 0..timing.tick(2.).unwrap() {
                let expected = spans
                    .iter()
                    .find(|s| {
                        timing.tick(s.start_seconds).unwrap() <= tick
                            && tick < timing.tick(s.end_seconds).unwrap()
                    })
                    .map(|s| s.midi);
                let actual = chart
                    .notes
                    .iter()
                    .find(|n| n.start <= tick && tick < n.end)
                    .map(|n| match n.tone {
                        VocalTone::Pitched { midi } => midi,
                        _ => panic!("No talkie substitution"),
                    });
                assert_eq!(actual, expected, "coverage at tick {tick}");
            }
            assert!(chart.notes.iter().any(|n| n.lyric == Syllable::Missing));
            assert!(issues.iter().any(|i| i.reason == Reason::MissingLyrics));
            assert!(
                !chart
                    .notes
                    .iter()
                    .any(|n| matches!(&n.lyric, Syllable::Text { text, .. } if text == "short"))
            );
            if words.is_none() {
                assert!(chart.notes.iter().all(|n| n.lyric == Syllable::Missing));
            }
        }
    }
    #[test]
    fn repeated_word_is_a_new_owner_and_short_word_fragment_is_retained_flagged() {
        let mut timings = words();
        timings.words[0].end = Some(0.7);
        timings.words.push(crate::clean::AlignedWord {
            text: Some("stay".into()),
            start: Some(0.8),
            end: Some(0.837),
        });
        let spans = [contour(0.2, 0.7, 57), contour(0.8, 1.1, 57)];
        let (chart, issues) = assemble(&spans, &[], &timings);
        assert_eq!(chart.notes.len(), 3);
        assert!(
            chart
                .notes
                .iter()
                .take(2)
                .all(|note| matches!(&note.lyric, Syllable::Text { text, .. } if text == "stay"))
        );
        assert_eq!(chart.notes[2].lyric, Syllable::Missing);
        assert_eq!(chart.notes[2].start, chart.notes[1].end);
        assert_eq!(
            chart.notes[2].end,
            TempoMap::constant(120.).unwrap().tick(1.1).unwrap()
        );
        let timing = TempoMap::constant(120.).unwrap();
        assert_eq!(chart.notes[1].end, timing.tick(0.837).unwrap());
        assert!(
            issues
                .iter()
                .any(|issue| issue.reason == Reason::ShortWordFragment
                    && issue.start_seconds == 0.8
                    && issue.end_seconds == 0.837)
        );
        assert!(
            !issues
                .iter()
                .any(|issue| issue.reason == Reason::UnsupportedPitchGap)
        );
    }
    #[test]
    fn active_coverage_at_phrase_break_is_flagged_without_merging_or_padding() {
        let mut timings = words();
        timings.words[0].end = Some(0.5);
        timings.words.push(crate::clean::AlignedWord {
            text: Some("again".into()),
            start: Some(1.5),
            end: Some(1.8),
        });
        let spans = [contour(0.2, 0.5, 57), contour(1.5, 1.8, 59)];
        let (silent, silent_issues) = assemble(&spans, &[], &timings);
        assert_eq!(silent.phrases.len(), 2);
        assert!(
            !silent_issues
                .iter()
                .any(|issue| issue.reason == Reason::CoveragePhraseBoundary)
        );
        for accepted in [false, true] {
            let mut frames = frames();
            for frame in &mut frames {
                frame.midi = accepted.then_some(57.);
                frame.local_energy_active = Some(true);
            }
            let (active, issues) = assemble(&spans, &frames, &timings);
            assert_eq!(active.notes, silent.notes);
            assert_eq!(active.phrases, silent.phrases);
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.reason == Reason::CoveragePhraseBoundary
                        && issue.start_seconds == 0.5
                        && issue.end_seconds == 1.5)
            );
            assert!(matches!(active.notes[1].lyric, Syllable::Text { .. }));
        }
    }
    #[test]
    fn final_hop_diagnostics_stay_within_source_clock_and_attach_to_review_map() {
        let duration = 2.001;
        let signal: Vec<f32> = (0..(duration * RATE as f64) as usize)
            .map(|i| (0.2 * (std::f64::consts::TAU * 220. * i as f64 / RATE as f64).sin()) as f32)
            .collect();
        let mut timings = words();
        timings.duration_seconds = duration;
        let mut report = generate_pcm(
            &signal,
            &"a".repeat(64),
            &"b".repeat(64),
            duration,
            &TempoMap::constant(120.).unwrap(),
            Some(&timings),
            None,
            &Policy::default(),
        )
        .unwrap();
        assert!(report.chart.is_some());
        let start = (duration * RATE as f64 / HOP as f64).floor() * HOP as f64 / RATE as f64;
        issue(
            &mut report.issues,
            start,
            start + HOP as f64 / RATE as f64,
            Reason::PitchDisagreement,
            "Last measured frame; hop extends past EOF",
        );
        assert!(crate::vocal_review::VocalReviewMap::from_generation(&report).is_err());
        let notes = serde_json::to_value(&report.chart).unwrap();
        bound_issue_ranges(&mut report.issues, duration);
        assert!(
            report
                .issues
                .iter()
                .all(|i| i.start_seconds < duration && i.end_seconds <= duration)
        );
        crate::vocal_review::VocalReviewMap::from_generation(&report).unwrap();
        assert_eq!(serde_json::to_value(&report.chart).unwrap(), notes);
    }
    use anyhow::Context;
    fn swift_fixture() -> crate::vocal_pitch::Evidence {
        crate::vocal_pitch::Evidence {
            version: 1,
            model_id: crate::vocal_pitch::MODEL_ID.into(),
            model_revision: crate::vocal_pitch::REVISION.into(),
            model_sha256: crate::vocal_pitch::MODEL_SHA256.into(),
            implementation_sha256: crate::vocal_pitch::implementation_hash(),
            audio_sha256: "a".repeat(64),
            vocal_stem_sha256: "b".repeat(64),
            pcm_sha256: "c".repeat(64),
            duration_seconds: 2.,
            sample_rate: 16000,
            hop_samples: 256,
            frames: (0..125)
                .map(|i| crate::vocal_pitch::Frame {
                    time_seconds: i as f64 * 0.016,
                    frequency_hz: Some(220.),
                    confidence: 0.9,
                    rms: 0.14,
                })
                .collect(),
        }
    }
    #[test]
    fn vocal_frame_evidence_rejects_wrong_sources_models_clock_and_nonfinite_frames() {
        let valid = swift_fixture();
        validate_vocal_frame_evidence(&valid, &"a".repeat(64), &"b".repeat(64), 2.).unwrap();
        for mode in 0..22 {
            let mut e = valid.clone();
            match mode {
                0 => e.audio_sha256 = "d".repeat(64),
                1 => e.vocal_stem_sha256 = "d".repeat(64),
                2 => e.model_id = "other".into(),
                3 => e.model_revision = "other".into(),
                4 => e.model_sha256 = "d".repeat(64),
                5 => e.implementation_sha256 = "d".repeat(64),
                6 => e.pcm_sha256 = "not a digest".into(),
                7 => e.sample_rate = 22050,
                8 => e.hop_samples = 160,
                9 => e.duration_seconds = 2.001,
                10 => e.duration_seconds = f64::NAN,
                11 => e.frames.clear(),
                12 => {
                    e.frames.truncate(120);
                }
                13 => e.frames[40].time_seconds += 0.001,
                14 => e.frames[40].confidence = f32::NAN,
                15 => e.frames[40].confidence = 1.1,
                16 => e.frames[40].rms = f64::NAN,
                17 => e.frames[40].rms = -0.1,
                18 => e.frames[40].frequency_hz = Some(f64::NAN),
                19 => e.frames[40].frequency_hz = Some(-1.),
                20 => e.frames[40].frequency_hz = Some(5000.),
                _ => e.version = 2,
            }
            assert!(
                validate_vocal_frame_evidence(&e, &"a".repeat(64), &"b".repeat(64), 2.).is_err(),
                "Invalid vocal evidence case{mode}"
            );
        }
        let mut invalid = valid.clone();
        invalid.audio_sha256 = "f".repeat(64);
        assert!(
            generate_pcm_with_vocal_pitch(
                &sine(),
                &"a".repeat(64),
                &"b".repeat(64),
                2.,
                &TempoMap::constant(120.).unwrap(),
                Some(&words()),
                None,
                &Policy::default(),
                None,
                &invalid
            )
            .is_err()
        );
    }
    #[test]
    fn file_entry_checks_actual_model_pcm_even_when_file_identity_matches() {
        let temp = tempfile::tempdir().unwrap();
        let audio = temp.path().join("voice.wav");
        let samples: Vec<_> = (0..88200)
            .map(|i| 0.2 * (std::f32::consts::TAU * 220. * i as f32 / 44100.).sin())
            .collect();
        models::write_audio(&audio, &samples, &samples).unwrap();
        let source = models::hash_file(&audio).unwrap();
        let mut e = swift_fixture();
        e.audio_sha256 = source.clone();
        e.vocal_stem_sha256 = source.clone();
        let binding = StemAlignment {
            audio_sha256: source.clone(),
            stem_sha256: source,
            evidence: "Known same synthetic source; intentionally incorrect inference PCM witness"
                .into(),
        };
        let error = generate(Request {
            audio: &audio,
            vocal_stem: &audio,
            alignment: &binding,
            timing: &TempoMap::constant(120.).unwrap(),
            words: None,
            lyrics: None,
            policy: &Policy::default(),
            neural_pitch: None,
            vocal_pitch: Some(&e),
        })
        .unwrap_err();
        assert!(error.to_string().contains("decoded source PCM"));
    }
    #[test]
    fn swift_recovery_requires_acoustic_word_and_model_evidence_and_keeps_flags() {
        let signal = sine();
        let mut f = yin_only();
        for frame in &mut f {
            frame.yin_midi = None;
        }
        let mut issues = vec![];
        let stages = recover_swift_frames(
            &mut f,
            &signal,
            Some(&words()),
            &swift_fixture(),
            0.0015,
            &mut issues,
        );
        assert!(stages.recovered > 140);
        assert!(
            f.iter()
                .filter(|f| f.midi.is_some())
                .all(|f| f.support == PitchSupport::SwiftF0 && f.clarity <= 0.5)
        );
        assert!(issues.iter().all(
            |i| i.reason == Reason::PitchDisagreement && i.detail.contains("SwiftF0 recovery")
        ));
        for mode in 0..9 {
            let mut signal = sine();
            let mut f = yin_only();
            let mut evidence = swift_fixture();
            let mut w = words();
            for frame in &mut f {
                frame.yin_midi = None;
            }
            match mode {
                0 => signal.fill(0.),
                1 => {
                    let mut seed = 789u32;
                    for v in &mut signal {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        *v = (seed as f64 / u32::MAX as f64 * 0.4 - 0.2) as f32;
                    }
                }
                2 => {
                    for frame in &mut f {
                        frame.mcleod_midi = Some(59.);
                    }
                }
                3 => {
                    for frame in &mut f {
                        frame.yin_midi = Some(69.);
                    }
                }
                4 => {
                    for frame in &mut evidence.frames {
                        frame.confidence = 0.79;
                    }
                }
                5 => w.words.clear(),
                6 => {
                    for (i, v) in signal.iter_mut().enumerate() {
                        *v = (0.2 * (std::f64::consts::TAU * 440. * i as f64 / RATE as f64).sin())
                            as f32;
                    }
                }
                7 => {
                    for frame in &mut f {
                        frame.local_energy_active = Some(false);
                    }
                }
                _ => {
                    for frame in &mut f {
                        frame.yin_midi = Some(56.6);
                        frame.mcleod_midi = Some(57.4);
                    }
                }
            }
            let mut issues = vec![];
            let stages =
                recover_swift_frames(&mut f, &signal, Some(&w), &evidence, 0.0015, &mut issues);
            assert_eq!(stages.recovered, 0, "Unsafe Swift recovery mode{mode}");
            assert!(issues.is_empty());
        }
    }
    #[test]
    #[ignore = "post-inference development neural/dropout diagnosis; no production recovery"]
    fn inspect_neural_period_dropout_evidence() -> Result<()> {
        let input = std::env::var("CHART_ENGINE_VOCAL_BASELINE")?;
        let report: Generation = serde_json::from_slice(&std::fs::read(input)?)?;
        let stem = std::env::var("CHART_ENGINE_VOCAL_STEM")?;
        ensure!(
            models::hash_file(Path::new(&stem))? == report.vocal_stem_sha256,
            "Stem identity mismatch"
        );
        let signal = models::decode(Path::new(&stem), RATE as u32, 1)?;
        let reference = crate::benchmark::parse(
            &std::fs::read(std::env::var("CHART_ENGINE_VOCAL_REFERENCE_MIDI")?)?,
            crate::benchmark::Instrument::Vocals,
            0.,
        )?;
        let neural = report.neural_pitch.as_ref().context("Neural evidence")?;
        for score in [0.6, 0.7, 0.8] {
            for correlation in [0.75, 0.85] {
                let mut counts = std::collections::BTreeMap::<String, usize>::new();
                for frame in &report.pitch_frames {
                    if frame.midi.is_some()
                        || frame.yin_midi.is_some()
                        || frame.mcleod_midi.is_some()
                    {
                        continue;
                    }
                    if !report.words.as_ref().is_some_and(|w| {
                        w.words.iter().any(|w| {
                            w.start.zip(w.end).is_some_and(|(s, e)| {
                                s <= frame.time_seconds && frame.time_seconds < e
                            })
                        })
                    }) {
                        continue;
                    }
                    let active: Vec<_> = neural
                        .notes
                        .iter()
                        .filter(|n| n.start <= frame.time_seconds && frame.time_seconds < n.end)
                        .collect();
                    for note in active.iter().filter(|n| n.confidence >= score) {
                        if active.iter().any(|other| {
                            other.pitch != note.pitch
                                && ![12, 19, 24, 28, 31]
                                    .contains(&(other.pitch as i16 - note.pitch as i16))
                                && note.confidence - other.confidence < 0.15
                        }) {
                            continue;
                        }
                        let center = (frame.time_seconds * RATE as f64).round() as usize;
                        let support = period_support(&signal, center, note.pitch as f64);
                        if support < correlation {
                            continue;
                        }
                        let shorter = [12., 19.01955000865, 24.]
                            .into_iter()
                            .map(|d| period_support(&signal, center, note.pitch as f64 + d))
                            .fold(-1., f64::max);
                        if support - shorter < 0.15 {
                            continue;
                        }
                        let expected = reference
                            .notes
                            .iter()
                            .find(|n| {
                                n.start + 0.04 <= frame.time_seconds
                                    && frame.time_seconds < n.end - 0.04
                            })
                            .and_then(|n| n.pitch);
                        let category = match expected {
                            Some(p) if p == note.pitch => "reference_correct",
                            Some(_) => "reference_wrong",
                            None => "outside_reference_interior",
                        };
                        *counts.entry(category.into()).or_default() += 1;
                    }
                }
                eprintln!(
                    "NN-only period proposal score{score},corr{correlation}: {}",
                    serde_json::to_string(&counts)?
                );
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "source-bound development-only short-window pitch diagnostic; no production recovery"]
    fn inspect_short_window_dropout_evidence() -> Result<()> {
        let report: Generation = serde_json::from_slice(&std::fs::read(std::env::var(
            "CHART_ENGINE_VOCAL_BASELINE",
        )?)?)?;
        let stem = std::env::var("CHART_ENGINE_VOCAL_STEM")?;
        ensure!(
            models::hash_file(Path::new(&stem))? == report.vocal_stem_sha256,
            "Stem identity mismatch"
        );
        let signal = models::decode(Path::new(&stem), RATE as u32, 1)?;
        let reference = crate::benchmark::parse(
            &std::fs::read(std::env::var("CHART_ENGINE_VOCAL_REFERENCE_MIDI")?)?,
            crate::benchmark::Instrument::Vocals,
            0.,
        )?;
        let neural = report.neural_pitch.as_ref().context("Neural evidence")?;
        let mut yin = YINDetector::<f64>::new(1024, 512);
        let mut mcleod = McLeodDetector::<f64>::new(1024, 512);
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for frame in &report.pitch_frames {
            if frame.midi.is_some() || frame.yin_midi.is_some() || frame.mcleod_midi.is_some() {
                continue;
            }
            if !report.words.as_ref().is_some_and(|w| {
                w.words.iter().any(|w| {
                    w.start
                        .zip(w.end)
                        .is_some_and(|(s, e)| s <= frame.time_seconds && frame.time_seconds < e)
                })
            }) {
                continue;
            }
            let center = (frame.time_seconds * RATE as f64).round() as usize;
            if center < 512 || center + 512 > signal.len() {
                continue;
            }
            let mut buffer: Vec<_> = signal[center - 512..center + 512]
                .iter()
                .map(|v| *v as f64)
                .collect();
            let mean = buffer.iter().sum::<f64>() / 1024.;
            for v in &mut buffer {
                *v -= mean;
            }
            let (Some(y), Some(m)) = (
                yin.get_pitch(&buffer, RATE, 0.00001, 0.8),
                mcleod.get_pitch(&buffer, RATE, 0.00001, 0.85),
            ) else {
                continue;
            };
            let y = 69. + 12. * (y.frequency / 440.).log2();
            let m = 69. + 12. * (m.frequency / 440.).log2();
            if !(35.5..84.5).contains(&y) || (y - m).abs() > 0.55 {
                continue;
            }
            let pitch = (y + m) / 2.;
            let active: Vec<_> = neural
                .notes
                .iter()
                .filter(|n| n.start <= frame.time_seconds && frame.time_seconds < n.end)
                .collect();
            let Some(note) = active
                .iter()
                .filter(|n| n.confidence >= 0.6 && (n.pitch as f64 - pitch).abs() <= 0.5)
                .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
            else {
                continue;
            };
            if active.iter().any(|other| {
                other.pitch != note.pitch
                    && ![12, 19, 24, 28, 31].contains(&(other.pitch as i16 - note.pitch as i16))
                    && note.confidence - other.confidence < 0.15
            }) {
                continue;
            }
            let support = period_support_window(&signal, center, pitch, 1024);
            let shorter = [12., 19.01955000865, 24.]
                .into_iter()
                .map(|d| period_support_window(&signal, center, pitch + d, 1024))
                .fold(-1., f64::max);
            if support < 0.85 || support - shorter < 0.15 {
                continue;
            }
            let expected = reference
                .notes
                .iter()
                .find(|n| n.start + 0.04 <= frame.time_seconds && frame.time_seconds < n.end - 0.04)
                .and_then(|n| n.pitch);
            let category = match expected {
                Some(p) if (p as f64 - pitch).abs() <= 0.6 => "reference_correct",
                Some(_) => "reference_wrong",
                None => "outside_reference_interior",
            };
            *counts.entry(category.into()).or_default() += 1;
        }
        eprintln!(
            "short-window dual+NN+direct-period: {}",
            serde_json::to_string(&counts)?
        );
        Ok(())
    }
    #[test]
    #[ignore = "development-only one-sided voiced anchor diagnostic; no production recovery"]
    fn inspect_one_sided_anchor_evidence() -> Result<()> {
        let report: Generation = serde_json::from_slice(&std::fs::read(std::env::var(
            "CHART_ENGINE_VOCAL_BASELINE",
        )?)?)?;
        let stem = std::env::var("CHART_ENGINE_VOCAL_STEM")?;
        ensure!(
            models::hash_file(Path::new(&stem))? == report.vocal_stem_sha256,
            "Stem identity mismatch"
        );
        let signal = models::decode(Path::new(&stem), RATE as u32, 1)?;
        let reference = crate::benchmark::parse(
            &std::fs::read(std::env::var("CHART_ENGINE_VOCAL_REFERENCE_MIDI")?)?,
            crate::benchmark::Instrument::Vocals,
            0.,
        )?;
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for (i, frame) in report.pitch_frames.iter().enumerate() {
            let Some(yin) = frame.yin_midi else {
                continue;
            };
            if frame.midi.is_some() || frame.mcleod_midi.is_some() || !(35.5..84.5).contains(&yin) {
                continue;
            }
            if !report.words.as_ref().is_some_and(|w| {
                w.words.iter().any(|w| {
                    w.start
                        .zip(w.end)
                        .is_some_and(|(s, e)| s <= frame.time_seconds && frame.time_seconds < e)
                })
            }) {
                continue;
            }
            let mut near: Vec<_> = report.pitch_frames
                [i.saturating_sub(2)..(i + 3).min(report.pitch_frames.len())]
                .iter()
                .filter_map(|f| f.yin_midi)
                .collect();
            near.sort_by(f64::total_cmp);
            if near.len() < 4
                || near.last().unwrap() - near.first().unwrap() > 0.7
                || (near[near.len() / 2] - yin).abs() > 0.4
            {
                continue;
            }
            let mut anchored = false;
            for j in i.saturating_sub(30)..(i + 31).min(report.pitch_frames.len()) {
                let anchor = &report.pitch_frames[j];
                if anchor.support != PitchSupport::Consensus
                    || anchor.midi.is_none_or(|m| (m - yin).abs() > 0.5)
                {
                    continue;
                }
                if report.pitch_frames[j.saturating_sub(2)..(j + 3).min(report.pitch_frames.len())]
                    .iter()
                    .filter(|f| {
                        f.support == PitchSupport::Consensus
                            && f.midi.is_some_and(|m| (m - yin).abs() <= 0.5)
                    })
                    .count()
                    < 3
                {
                    continue;
                }
                if report.pitch_frames[i.min(j)..=i.max(j)].iter().all(|f| {
                    f.yin_midi.is_some_and(|y| (y - yin).abs() <= 0.7)
                        && f.mcleod_midi.is_none_or(|m| (m - yin).abs() <= 0.55)
                }) {
                    anchored = true;
                    break;
                }
            }
            if !anchored {
                continue;
            }
            let center = (frame.time_seconds * RATE as f64).round() as usize;
            let support = period_support(&signal, center, yin);
            let shorter = [12., 19.01955000865, 24.]
                .into_iter()
                .map(|d| period_support(&signal, center, yin + d))
                .fold(-1., f64::max);
            if support < 0.85 || support - shorter < 0.15 {
                continue;
            }
            let expected = reference
                .notes
                .iter()
                .find(|n| n.start + 0.04 <= frame.time_seconds && frame.time_seconds < n.end - 0.04)
                .and_then(|n| n.pitch);
            let category = match expected {
                Some(p) if (p as f64 - yin).abs() <= 0.6 => "reference_correct",
                Some(_) => "reference_wrong",
                None => "outside_reference_interior",
            };
            *counts.entry(category.into()).or_default() += 1;
        }
        eprintln!(
            "one-sided same-pitch contiguous voiced anchors: {}",
            serde_json::to_string(&counts)?
        );
        Ok(())
    }
    #[test]
    fn older_reports_keep_strict_policy_and_unknown_frame_support() {
        let mut policy = serde_json::to_value(Policy::default()).unwrap();
        for rejected in ["recover_contour_tails", "recover_short_consensus"] {
            let mut legacy = policy.clone();
            legacy[rejected] = serde_json::json!(false);
            let strict: Policy = serde_json::from_value(legacy.clone()).unwrap();
            assert!(
                serde_json::to_value(strict)
                    .unwrap()
                    .get(rejected)
                    .is_none()
            );
            legacy[rejected] = serde_json::json!(true);
            assert!(serde_json::from_value::<Policy>(legacy).is_err());
        }
        policy
            .as_object_mut()
            .unwrap()
            .remove("recover_bounded_yin");
        policy
            .as_object_mut()
            .unwrap()
            .remove("recover_contour_tails");
        policy
            .as_object_mut()
            .unwrap()
            .remove("recover_short_consensus");
        policy.as_object_mut().unwrap().remove("local_energy_gate");
        let old: Policy = serde_json::from_value(policy).unwrap();
        assert!(!old.recover_bounded_yin);
        assert!(!old.local_energy_gate);
        assert!(Policy::default().recover_bounded_yin);
        let old: PitchFrame = serde_json::from_value(serde_json::json!({
            "time_seconds":1.0,"midi":57.0,"clarity":0.9,"rms":0.1
        }))
        .unwrap();
        assert_eq!(old.support, PitchSupport::Unspecified);
        assert!(old.yin_midi.is_none() && old.mcleod_midi.is_none());
        let old: LyricSource = serde_json::from_value(serde_json::json!({
            "text":"stay","source":"Explicit fixture"
        }))
        .unwrap();
        assert!(old.synced_lyrics.is_none());
    }
    fn neural() -> NeuralPitchEvidence {
        NeuralPitchEvidence {
            audio_sha256: "a".repeat(64),
            vocal_stem_sha256: "b".repeat(64),
            duration_seconds: 2.,
            model_sha256: models::assets()
                .into_iter()
                .find(|a| a.id == "pitch")
                .unwrap()
                .sha256,
            implementation_sha256: neural_pitch_implementation_hash(),
            notes: vec![
                crate::mapping::PitchNote {
                    start: 0.2,
                    end: 1.8,
                    pitch: 57,
                    confidence: 0.8,
                },
                crate::mapping::PitchNote {
                    start: 0.2,
                    end: 1.8,
                    pitch: 69,
                    confidence: 0.9,
                },
            ],
        }
    }
    fn yin_only() -> Vec<PitchFrame> {
        frames()
            .into_iter()
            .map(|mut f| {
                f.midi = None;
                f.mcleod_midi = None;
                f.support = PitchSupport::Unspecified;
                f
            })
            .collect()
    }
    #[test]
    fn neural_and_yin_can_recover_unanchored_voice_while_retaining_harmonic_uncertainty() {
        let mut frames = yin_only();
        let mut issues = Vec::new();
        recover_neural_yin(&mut frames, &sine(), Some(&words()), &neural(), &mut issues);
        let recovered: Vec<_> = frames.iter().filter(|f| f.midi.is_some()).collect();
        assert!(recovered.len() > 140);
        assert!(
            recovered
                .iter()
                .all(|f| f.support == PitchSupport::NeuralYin
                    && f.mcleod_midi.is_none()
                    && f.yin_midi == Some(57.)
                    && f.midi == Some(57.)
                    && f.clarity <= 0.5)
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].reason, Reason::PitchDisagreement);
        assert!(issues[0].detail.contains("not musical approval"));
    }
    #[test]
    fn neural_vote_cannot_rescue_noise_wrong_octave_bleed_conflicts_ambiguity_or_unknown_words() {
        for mode in 0..10 {
            let mut frames = yin_only();
            let mut signal = sine();
            let mut neural = neural();
            let mut words = words();
            match mode {
                0 => {
                    let mut seed = 987u32;
                    for v in &mut signal {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        *v = (seed as f64 / u32::MAX as f64 * 0.4 - 0.2) as f32;
                    }
                }
                1 | 2 => {
                    let frequency = if mode == 1 { 440. } else { 330. };
                    for (i, v) in signal.iter_mut().enumerate() {
                        *v = (0.2
                            * (std::f64::consts::TAU * frequency * i as f64 / RATE as f64).sin())
                            as f32;
                    }
                }
                3 => {
                    for f in &mut frames {
                        f.mcleod_midi = Some(69.);
                    }
                }
                4 | 5 => neural.notes.push(crate::mapping::PitchNote {
                    start: 0.,
                    end: 2.,
                    pitch: if mode == 4 { 59 } else { 45 },
                    confidence: 0.75,
                }),
                6 => words.words.clear(),
                7 => {
                    for f in &mut frames {
                        f.yin_midi = None;
                    }
                }
                8 => neural.notes[0].pitch = 59,
                _ => neural.notes[0].confidence = 0.59,
            }
            let mut issues = Vec::new();
            recover_neural_yin(&mut frames, &signal, Some(&words), &neural, &mut issues);
            assert!(
                frames.iter().all(|f| f.midi.is_none()),
                "Unsafe neural recovery case {mode}"
            );
            assert!(issues.is_empty());
        }
    }
    #[test]
    fn neural_evidence_is_bound_to_recording_model_implementation_clock_and_valid_bounds() {
        let signal = sine();
        let map = TempoMap::constant(120.).unwrap();
        let words = words();
        for mode in 0..6 {
            let mut evidence = neural();
            match mode {
                0 => evidence.audio_sha256 = "c".repeat(64),
                1 => evidence.model_sha256 = "c".repeat(64),
                2 => evidence.implementation_sha256 = "c".repeat(64),
                3 => evidence.duration_seconds = 2.001,
                4 => evidence.notes[0].end = 2.01,
                _ => evidence.notes[0].confidence = f32::NAN,
            }
            assert!(
                generate_pcm_with_neural_pitch(
                    &signal,
                    &"a".repeat(64),
                    &"b".repeat(64),
                    2.,
                    &map,
                    Some(&words),
                    None,
                    &Policy::default(),
                    &evidence
                )
                .is_err()
            );
        }
    }
    fn sine() -> Vec<f32> {
        (0..RATE * 2)
            .map(|i| (0.2 * (std::f64::consts::TAU * 220. * i as f64 / RATE as f64).sin()) as f32)
            .collect()
    }
    fn frames() -> Vec<PitchFrame> {
        (0..200)
            .map(|i| PitchFrame {
                time_seconds: i as f64 * HOP as f64 / RATE as f64,
                midi: Some(57.),
                yin_midi: Some(57.),
                mcleod_midi: Some(57.),
                clarity: 0.99,
                rms: 0.14,
                local_rms: None,
                local_energy_active: None,
                support: PitchSupport::Consensus,
            })
            .collect()
    }
    fn words() -> WordTimings {
        WordTimings {
            version: 1,
            audio_sha256: "a".repeat(64),
            duration_seconds: 2.,
            source: "Known synthetic sung word".into(),
            words: vec![crate::clean::AlignedWord {
                text: Some("stay".into()),
                start: Some(0.2),
                end: Some(1.8),
            }],
        }
    }
    fn dropout(frames: &mut [PitchFrame]) {
        for frame in &mut frames[70..105] {
            frame.midi = None;
            frame.mcleod_midi = None;
            frame.support = PitchSupport::Unspecified;
        }
    }
    #[test]
    fn anchored_periodic_dropout_recovers_pitch_and_keeps_uncertainty() {
        let mut frames = frames();
        dropout(&mut frames);
        let mut issues = Vec::new();
        recover_yin_gaps(&mut frames, &sine(), Some(&words()), &mut issues);
        assert!(frames[70..105].iter().all(|frame| frame.midi == Some(57.)
            && frame.mcleod_midi.is_none()
            && frame.support == PitchSupport::AnchoredYin));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].reason, Reason::PitchDisagreement);
        assert!(issues[0].detail.contains("McLeod did not provide"));
    }
    #[test]
    fn recovery_rejects_unanchored_missing_word_octave_conflict_noise_and_bleed() {
        let original = frames();
        for mode in 0..6 {
            let mut frames = original.clone();
            dropout(&mut frames);
            let mut audio = sine();
            let mut words = words();
            match mode {
                0 => {
                    for frame in &mut frames {
                        frame.midi = None;
                        frame.mcleod_midi = None;
                    }
                }
                1 => words.words.clear(),
                2 => {
                    for frame in &mut frames[70..105] {
                        frame.yin_midi = Some(69.);
                    }
                }
                3 => {
                    for frame in &mut frames[70..105] {
                        frame.mcleod_midi = Some(69.);
                    }
                }
                4 => {
                    let mut seed = 12345u32;
                    for sample in &mut audio {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        *sample = (seed as f64 / u32::MAX as f64 * 0.4 - 0.2) as f32;
                    }
                }
                _ => {
                    for (i, sample) in audio.iter_mut().enumerate() {
                        *sample = (0.2
                            * (std::f64::consts::TAU * 330. * i as f64 / RATE as f64).sin())
                            as f32;
                    }
                }
            }
            let mut issues = Vec::new();
            recover_yin_gaps(&mut frames, &audio, Some(&words), &mut issues);
            assert!(
                frames[70..105].iter().all(|frame| frame.midi.is_none()),
                "unsafe recovery in case {mode}"
            );
            assert!(issues.is_empty());
        }
    }
    #[test]
    #[ignore = "requires explicit local vocal baseline and stem"]
    fn inspect_local_yin_only_gaps() {
        let baseline: Generation = serde_json::from_slice(
            &std::fs::read(std::env::var("CHART_ENGINE_VOCAL_BASELINE").unwrap()).unwrap(),
        )
        .unwrap();
        let signal = models::decode(
            Path::new(&std::env::var("CHART_ENGINE_VOCAL_STEM").unwrap()),
            RATE as u32,
            1,
        )
        .unwrap();
        let frames = &baseline.pitch_frames;
        let mut eligible = Vec::new();
        let mut widths = Vec::new();
        let mut index = 0;
        while index < frames.len() {
            if frames[index].midi.is_some() {
                index += 1;
                continue;
            }
            let start = index;
            while index < frames.len() && frames[index].midi.is_none() {
                index += 1;
            }
            if start == 0 || index == frames.len() {
                continue;
            }
            let (Some(before), Some(after)) = (frames[start - 1].midi, frames[index].midi) else {
                continue;
            };
            if (before - after).abs() > 7.0 {
                continue;
            }
            widths.push((index - start) as f64 * HOP as f64 / RATE as f64);
            if index - start > 80 {
                continue;
            }
            for frame in &frames[start..index] {
                let Some(yin) = frame.yin_midi else {
                    continue;
                };
                if yin < before.min(after) - 2.0 || yin > before.max(after) + 2.0 {
                    continue;
                }
                if !baseline.words.as_ref().is_some_and(|words| {
                    words.words.iter().any(|word| {
                        word.start
                            .zip(word.end)
                            .is_some_and(|(a, b)| a <= frame.time_seconds && frame.time_seconds < b)
                    })
                }) {
                    continue;
                }
                eligible.push(period_support(
                    &signal,
                    (frame.time_seconds * RATE as f64).round() as usize,
                    yin,
                ));
            }
        }
        widths.sort_by(f64::total_cmp);
        eligible.sort_by(f64::total_cmp);
        if widths.is_empty() || eligible.is_empty() {
            eprintln!("No bounded YIN-only frames with timed-word context in this recording");
            return;
        }
        eprintln!(
            "bounded contour gaps {}; gap widths {:?}",
            widths.len(),
            [0.1, 0.5, 0.9].map(|q| widths[(widths.len() as f64 * q) as usize])
        );
        eprintln!(
            "eligible frames {} ({:.2}s), correlation quantiles {:?}, >=0.90 {} frames >=0.85 {} frames",
            eligible.len(),
            eligible.len() as f64 * HOP as f64 / RATE as f64,
            [0.1, 0.5, 0.9].map(|q| eligible[(eligible.len() as f64 * q) as usize]),
            eligible.iter().filter(|v| **v >= 0.9).count(),
            eligible.iter().filter(|v| **v >= 0.85).count()
        );
    }

    #[test]
    #[ignore = "requires explicit source-bound vocal baseline, stem and verified Basic Pitch model"]
    fn inspect_neural_vocal_second_vote() -> Result<()> {
        let path = |key: &str| -> Result<std::path::PathBuf> { Ok(std::env::var(key)?.into()) };
        let baseline: Generation =
            serde_json::from_slice(&std::fs::read(path("CHART_ENGINE_VOCAL_BASELINE")?)?)?;
        let stem = path("CHART_ENGINE_VOCAL_STEM")?;
        let root = path("CHART_ENGINE_VOCAL_MODELS")?;
        let output = path("CHART_ENGINE_VOCAL_OUTPUT")?;
        ensure!(!output.exists(), "Choose a new neural probe directory");
        ensure!(
            models::hash_file(&stem)? == baseline.vocal_stem_sha256,
            "Neural probe stem belongs to another recording"
        );
        let model = models::verified(&root, "pitch")?;
        let model_hash = models::hash_file(&model)?;
        let notes = models::pitch(&stem, &model)?;
        let signal = models::decode(&stem, RATE as u32, 1)?;
        let words = baseline
            .words
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Timed words required"))?;
        let mut staged = Vec::new();
        for (index, frame) in baseline.pitch_frames.iter().enumerate() {
            let Some(yin) = frame.yin_midi else {
                continue;
            };
            if frame.midi.is_some()
                || frame.mcleod_midi.is_some()
                || !(35.5..84.5).contains(&yin)
                || !words.words.iter().any(|w| {
                    w.text.is_some()
                        && w.start
                            .zip(w.end)
                            .is_some_and(|(a, b)| a <= frame.time_seconds && frame.time_seconds < b)
                })
            {
                continue;
            }
            let active: Vec<_> = notes
                .iter()
                .filter(|n| n.start <= frame.time_seconds && frame.time_seconds < n.end)
                .collect();
            let matching = active
                .iter()
                .copied()
                .filter(|n| (n.pitch as f64 - yin).abs() <= 0.5)
                .max_by(|a, b| a.confidence.total_cmp(&b.confidence));
            let Some(note) = matching else {
                continue;
            };
            let competitor = |ignore_harmonics: bool| {
                active
                    .iter()
                    .filter(|n| n.pitch != note.pitch)
                    .filter(|n| {
                        !ignore_harmonics
                            || ![12, 19, 24, 28, 31].contains(&(n.pitch as i16 - note.pitch as i16))
                    })
                    .map(|n| n.confidence)
                    .fold(0., f32::max)
            };
            let mut nearby: Vec<_> = baseline.pitch_frames
                [index.saturating_sub(2)..(index + 3).min(baseline.pitch_frames.len())]
                .iter()
                .filter_map(|f| f.yin_midi)
                .collect();
            nearby.sort_by(f64::total_cmp);
            let stable = nearby.len() >= 4
                && nearby.last().unwrap() - nearby.first().unwrap() <= 0.7
                && (nearby[nearby.len() / 2] - yin).abs() <= 0.4;
            let center = (frame.time_seconds * RATE as f64).round() as usize;
            let correlation = period_support(&signal, center, yin);
            let harmonic_correlation = [12., 19.01955000865, 24.]
                .map(|delta| period_support(&signal, center, yin + delta))
                .into_iter()
                .fold(-1., f64::max);
            staged.push(serde_json::json!({"time":frame.time_seconds,"yin":yin,"matching_score":note.confidence,
                "strongest_other":competitor(false),"strongest_nonharmonic":competitor(true),
                "stable":stable,"correlation":correlation,"fundamental_contrast":correlation-harmonic_correlation}));
        }
        let mut margin_counts = Vec::new();
        for score in [0.45, 0.5, 0.6] {
            for correlation in [0.65, 0.75, 0.85] {
                let matching =
                    |v: &&serde_json::Value| v["matching_score"].as_f64().unwrap() >= score;
                let stable = |v: &&serde_json::Value| matching(v) && v["stable"].as_bool().unwrap();
                let periodic = |v: &&serde_json::Value| {
                    stable(v) && v["correlation"].as_f64().unwrap() >= correlation
                };
                let margin = |v: &&serde_json::Value| {
                    periodic(v)
                        && v["matching_score"].as_f64().unwrap()
                            - v["strongest_nonharmonic"].as_f64().unwrap()
                            >= 0.15
                };
                let accepted: Vec<_> = staged
                    .iter()
                    .filter(margin)
                    .filter(|v| v["fundamental_contrast"].as_f64().unwrap() >= 0.15)
                    .cloned()
                    .collect();
                margin_counts.push(serde_json::json!({"score":score,"minimum_correlation":correlation,"matching_frames":staged.iter().filter(matching).count(),
                "stable_frames":staged.iter().filter(stable).count(),"periodic_frames":staged.iter().filter(periodic).count(),
                "nonharmonic_margin_frames":staged.iter().filter(margin).count(),"fundamental_frames":accepted.len(),
                "fundamental_seconds":accepted.len() as f64*HOP as f64/RATE as f64,"accepted":accepted}));
            }
        }
        let mut counts = Vec::new();
        for score in [0.6f32, 0.7, 0.8] {
            for correlation in [0.80, 0.85, 0.90] {
                let mut accepted = Vec::new();
                for (index, frame) in baseline.pitch_frames.iter().enumerate() {
                    let Some(yin) = frame.yin_midi else {
                        continue;
                    };
                    if frame.midi.is_some()
                        || frame.mcleod_midi.is_some()
                        || !(35.5..84.5).contains(&yin)
                        || !words.words.iter().any(|w| {
                            w.text.is_some()
                                && w.start.zip(w.end).is_some_and(|(a, b)| {
                                    a <= frame.time_seconds && frame.time_seconds < b
                                })
                        })
                    {
                        continue;
                    }
                    let active: Vec<_> = notes
                        .iter()
                        .filter(|note| {
                            note.start <= frame.time_seconds
                                && frame.time_seconds < note.end
                                && note.confidence >= 0.3
                        })
                        .collect();
                    let Some(note) = active.iter().copied().find(|note| {
                        note.confidence >= score && (note.pitch as f64 - yin).abs() <= 0.5
                    }) else {
                        continue;
                    };
                    if active.iter().any(|other| other.pitch != note.pitch) {
                        continue;
                    }
                    let mut nearby: Vec<_> = baseline.pitch_frames
                        [index.saturating_sub(2)..(index + 3).min(baseline.pitch_frames.len())]
                        .iter()
                        .filter_map(|f| f.yin_midi)
                        .collect();
                    if nearby.len() < 4 {
                        continue;
                    }
                    nearby.sort_by(f64::total_cmp);
                    if nearby.last().unwrap() - nearby.first().unwrap() > 0.7
                        || (nearby[nearby.len() / 2] - yin).abs() > 0.4
                    {
                        continue;
                    }
                    let support = period_support(
                        &signal,
                        (frame.time_seconds * RATE as f64).round() as usize,
                        yin,
                    );
                    if support >= correlation {
                        accepted.push(frame.time_seconds);
                    }
                }
                counts.push(serde_json::json!({"minimum_neural_note_score":score,
                    "minimum_period_correlation":correlation,"additional_frames":accepted.len(),
                    "additional_frame_seconds":accepted.len() as f64*HOP as f64/RATE as f64,
                    "times":accepted}));
            }
        }
        ensure!(
            models::hash_file(&stem)? == baseline.vocal_stem_sha256
                && models::hash_file(&model)? == model_hash,
            "Neural probe sources changed during inference"
        );
        std::fs::create_dir(&output)?;
        std::fs::write(
            output.join("neural-notes.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
            "audio_sha256":baseline.audio_sha256,"vocal_stem_sha256":baseline.vocal_stem_sha256,
            "model_sha256":model_hash,"duration_seconds":baseline.duration_seconds,"notes":notes}))?,
        )?;
        let summary = serde_json::json!({"neural_notes":notes.len(),"raw_note_seconds":notes.iter().map(|n| n.end-n.start).sum::<f64>(),"probes":counts,"margin_probes":margin_counts});
        std::fs::write(
            output.join("neural-recovery-probe.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
        for case in summary["probes"].as_array().unwrap() {
            eprintln!(
                "BasicPitch/YIN second vote score={} correlation={}: {} additional frames ({}s)",
                case["minimum_neural_note_score"],
                case["minimum_period_correlation"],
                case["additional_frames"],
                case["additional_frame_seconds"]
            );
        }
        Ok(())
    }
}
