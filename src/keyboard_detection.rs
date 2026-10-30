//! Keep keyboard detector results tied to their audio input.
//! Controller arrangement happens separately.
use crate::{
    keyboard_audio,
    mapping::{PitchCandidateEvidence, PitchNote, Transcription},
    models, muscriptor, piano_model,
    stem_review::{StemId, StemManifest},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardSound {
    #[default]
    Piano,
    Synth,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PianoQuality {
    #[default]
    Standard,
    HighQuality,
}

pub fn high_quality_piano(
    instruments: &[String],
    sound: KeyboardSound,
    quality: PianoQuality,
) -> bool {
    quality == PianoQuality::HighQuality
        && sound == KeyboardSound::Piano
        && instruments
            .iter()
            .any(|s| matches!(s.as_str(), "piano" | "pro_piano"))
}
impl KeyboardSound {
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Piano => "piano",
            Self::Synth => "pitch",
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::Piano => piano_model::METHOD,
            Self::Synth => {
                "basic-pitch-native-keyboard-v1;piano+other;unity-sum;22050Hz;onset-first+Melodia;source-clock"
            }
        }
    }
    pub fn score_semantics(self) -> &'static str {
        match self {
            Self::Piano => {
                "uncalibrated regression onset head at the detected attack; not a probability of note correctness"
            }
            Self::Synth => {
                "uncalibrated mean frame activation over the decoded note; not a probability of note correctness"
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub version: u32,
    pub sound: KeyboardSound,
    pub source_audio_sha256: String,
    pub duration_seconds: f64,
    pub stem_sha256: BTreeMap<String, String>,
    pub model_id: String,
    pub model_sha256: String,
    pub implementation_sha256: String,
    pub method: String,
    pub score_semantics: String,
    pub decoder_parameters: BTreeMap<String, f32>,
    pub raw_notes: Vec<PitchNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basic_pitch_candidates: Vec<PitchCandidateEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub velocities: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pedals: Vec<piano_model::Pedal>,
    /// v2: unfiltered instrument-aware output and exact arrangement selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muscriptor: Option<SynthEvidence>,
    /// v4 Piano: actual original-minus-drums/bass/vocals analysis recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub piano_audio: Option<keyboard_audio::Receipt>,
    /// v5: both complete detector outputs and exact corroborated selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub piano_multiview: Option<PianoViews>,
}
pub const PIANO_SELECTION_V5: &str = "piano-corroboration-v1: strong baseline onset>=0.65; residual requires same-PC strong baseline within750ms plus2 strong attack groups separated>=50ms within1500ms; retain strong baseline fallback lacking selected residual same-PC within50ms; source-clock; all raw voices retained; baseline pedals only";
pub const PIANO_SELECTION: &str = "piano-corroboration-v2: strong baseline onset>=0.65; residual requires same-PC strong baseline within750ms plus2 strong attack groups separated>=50ms within1500ms; maximum-cardinality one-to-one same-PC baseline/residual coverage within50ms in onset/index order; retain unmatched strong baseline attacks; source-clock; all raw voices retained; baseline pedals only";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PianoView {
    Baseline,
    Residual,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PianoIndex {
    pub view: PianoView,
    pub index: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PianoViews {
    pub baseline: piano_model::Output,
    pub residual: piano_model::Output,
    pub selection_policy: String,
    pub selected_indices: Vec<PianoIndex>,
}
pub fn piano_selected_indices(baseline: &[PitchNote], residual: &[PitchNote]) -> Vec<PianoIndex> {
    // Only local attacks can corroborate a residual voice. Keep the exact
    // absolute-distance comparisons at the inclusive time boundaries.
    let mut strong: Vec<_> = baseline.iter().filter(|n| n.confidence >= 0.65).collect();
    strong.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut selected: Vec<_> = residual
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            let low = strong.partition_point(|b| n.start - b.start > 1.5);
            let high = strong.partition_point(|b| b.start - n.start <= 1.5);
            let local = &strong[low..high];
            if !local
                .iter()
                .any(|b| b.pitch % 12 == n.pitch % 12 && (b.start - n.start).abs() <= 0.75)
            {
                return false;
            }
            let mut groups = 0;
            let mut last = f64::NEG_INFINITY;
            for b in local.iter().filter(|b| (b.start - n.start).abs() <= 1.5) {
                if b.start - last >= 0.05 {
                    groups += 1;
                    last = b.start;
                    if groups >= 2 {
                        return true;
                    }
                }
            }
            false
        })
        .map(|(index, _)| PianoIndex {
            view: PianoView::Residual,
            index,
        })
        .collect();
    let mut covered = vec![false; baseline.len()];
    for class in 0..12 {
        let mut base: Vec<_> = baseline
            .iter()
            .enumerate()
            .filter(|(_, n)| n.confidence >= 0.65 && n.pitch % 12 == class)
            .map(|(i, _)| i)
            .collect();
        let mut recovered: Vec<_> = selected
            .iter()
            .filter(|i| residual[i.index].pitch % 12 == class)
            .map(|i| i.index)
            .collect();
        base.sort_by(|&a, &b| {
            baseline[a]
                .start
                .total_cmp(&baseline[b].start)
                .then(a.cmp(&b))
        });
        recovered.sort_by(|&a, &b| {
            residual[a]
                .start
                .total_cmp(&residual[b].start)
                .then(a.cmp(&b))
        });
        let (mut left, mut right) = (0, 0);
        // Equal-radius ordered intervals admit an earliest-compatible greedy
        // maximum matching: crossing matched pairs can always be uncrossed.
        while left < base.len() && right < recovered.len() {
            let b = baseline[base[left]].start;
            let r = residual[recovered[right]].start;
            if (b - r).abs() <= 0.05 {
                covered[base[left]] = true;
                left += 1;
                right += 1;
            } else if b < r {
                left += 1;
            } else {
                right += 1;
            }
        }
    }
    selected.extend(
        baseline
            .iter()
            .enumerate()
            .filter(|(i, n)| n.confidence >= 0.65 && !covered[*i])
            .map(|(index, _)| PianoIndex {
                view: PianoView::Baseline,
                index,
            }),
    );
    selected.sort_by(|a, b| {
        let note = |i: &PianoIndex| match i.view {
            PianoView::Baseline => &baseline[i.index],
            PianoView::Residual => &residual[i.index],
        };
        note(a)
            .start
            .total_cmp(&note(b).start)
            .then(note(a).pitch.cmp(&note(b).pitch))
            .then(a.index.cmp(&b.index))
    });
    selected
}
/// Frozen v5 behavior. Never reinterpret historical selection evidence.
fn piano_selected_indices_v5(baseline: &[PitchNote], residual: &[PitchNote]) -> Vec<PianoIndex> {
    let strong: Vec<_> = baseline.iter().filter(|n| n.confidence >= 0.65).collect();
    let mut selected: Vec<_> = residual
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            if !strong
                .iter()
                .any(|b| b.pitch % 12 == n.pitch % 12 && (b.start - n.start).abs() <= 0.75)
            {
                return false;
            }
            let mut starts: Vec<_> = strong
                .iter()
                .filter(|b| (b.start - n.start).abs() <= 1.5)
                .map(|b| b.start)
                .collect();
            starts.sort_by(f64::total_cmp);
            let mut groups = 0;
            let mut last = f64::NEG_INFINITY;
            for onset in starts {
                if onset - last >= 0.05 {
                    groups += 1;
                    last = onset;
                }
            }
            groups >= 2
        })
        .map(|(index, _)| PianoIndex {
            view: PianoView::Residual,
            index,
        })
        .collect();
    let recovered = selected.clone();
    selected.extend(
        baseline
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.confidence >= 0.65
                    && !recovered.iter().any(|i| {
                        let r = &residual[i.index];
                        r.pitch % 12 == n.pitch % 12 && (r.start - n.start).abs() <= 0.05
                    })
            })
            .map(|(index, _)| PianoIndex {
                view: PianoView::Baseline,
                index,
            }),
    );
    selected.sort_by(|a, b| {
        let note = |i: &PianoIndex| match i.view {
            PianoView::Baseline => &baseline[i.index],
            PianoView::Residual => &residual[i.index],
        };
        note(a)
            .start
            .total_cmp(&note(b).start)
            .then(note(a).pitch.cmp(&note(b).pitch))
            .then(a.index.cmp(&b.index))
    });
    selected
}
fn selected_piano(views: &PianoViews) -> (Vec<PitchNote>, Vec<u8>) {
    views
        .selected_indices
        .iter()
        .map(|i| {
            let o = match i.view {
                PianoView::Baseline => &views.baseline,
                PianoView::Residual => &views.residual,
            };
            (o.notes[i.index].clone(), o.velocities[i.index])
        })
        .unzip()
}
pub fn piano_multiview_method() -> String {
    format!("{};{}", piano_method(), PIANO_SELECTION)
}
pub fn piano_method() -> String {
    format!("{};{}", piano_model::METHOD, keyboard_audio::RECIPE)
}
pub const SYNTH_SELECTION_V2: &str = "lead-harmony-v1: preserve all non-drum melodic voices except explicit GM bass programs32..39; instrument labels uncertain; conditioning group32 advisory";
pub const SYNTH_SELECTION: &str = "lead-harmony-v2: preserve non-bass non-drum voices; rescue suspect bass-labelled phrases only at MIDI48+ with notes>=40ms, links<=750ms/7semitones, >=3 attacks over>=200ms and >=2 MIDI60+ anchor attacks (attack groups>=50ms); never rescue drums or MIDI<48; labels uncertain; group32 advisory";
pub const SYNTH_PRIORITY: &str = "nominal arrangement priority1; not a model confidence or correctness probability; MuScriptor emits no note confidence";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthEvidence {
    pub output: muscriptor::Output,
    pub selection_policy: String,
    pub selected_indices: Vec<usize>,
}
fn legacy_synth_selected_indices(notes: &[muscriptor::Note]) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_drum && n.program != 128 && !(32..=39).contains(&n.program))
        .map(|(i, _)| i)
        .collect()
}
/// Bass labels are advisory, but their low register remains excluded. A linked
/// phrase with repeated middle-C-or-higher attacks is evidence for retaining
/// an upper keyboard candidate; it does not establish instrument identity.
/// Constants use C3/C4 register boundaries and local phrase continuity, never
/// song identity, reference notes, model "confidence" or target note counts.
pub fn synth_selected_indices(notes: &[muscriptor::Note]) -> Vec<usize> {
    let mut candidates: Vec<_> = notes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            !n.is_drum
                && (32..=39).contains(&n.program)
                && n.pitch >= 48
                && n.start.is_finite()
                && n.start >= 0.
                && n.end.is_finite()
                && n.end - n.start + 1e-9 >= 0.040
        })
        .map(|(i, _)| i)
        .collect();
    candidates.sort_by(|&a, &b| notes[a].start.total_cmp(&notes[b].start).then(a.cmp(&b)));
    fn root(parents: &mut [usize], mut node: usize) -> usize {
        while parents[node] != node {
            parents[node] = parents[parents[node]];
            node = parents[node];
        }
        node
    }
    let mut parents: Vec<_> = (0..candidates.len()).collect();
    let mut first = 0;
    for right in 0..candidates.len() {
        let n = &notes[candidates[right]];
        while first < right && n.start - notes[candidates[first]].start > 0.750 + 1e-9 {
            first += 1;
        }
        for left in first..right {
            if n.pitch.abs_diff(notes[candidates[left]].pitch) <= 7 {
                let a = root(&mut parents, left);
                let b = root(&mut parents, right);
                if a != b {
                    parents[b] = a;
                }
            }
        }
    }
    let mut phrases: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, &index) in candidates.iter().enumerate() {
        phrases
            .entry(root(&mut parents, i))
            .or_default()
            .push(index);
    }
    let distinct = |indices: &[usize], anchor: bool| -> usize {
        let mut last = None;
        let mut count = 0;
        for &i in indices {
            let n = &notes[i];
            if anchor && n.pitch < 60 {
                continue;
            }
            if last.is_none_or(|time: f64| n.start - time + 1e-9 >= 0.050) {
                last = Some(n.start);
                count += 1;
            }
        }
        count
    };
    let mut rescue = vec![false; notes.len()];
    for indices in phrases.values() {
        let span = notes[*indices.last().unwrap()].start - notes[indices[0]].start;
        if span + 1e-9 >= 0.200 && distinct(indices, false) >= 3 && distinct(indices, true) >= 2 {
            for &index in indices {
                rescue[index] = true;
            }
        }
    }
    notes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            !n.is_drum && n.program != 128 && (!(32..=39).contains(&n.program) || rescue[*i])
        })
        .map(|(i, _)| i)
        .collect()
}
fn synth_notes(output: &muscriptor::Output, indices: &[usize]) -> Vec<PitchNote> {
    indices
        .iter()
        .map(|&i| {
            let n = &output.notes[i];
            PitchNote {
                start: n.start,
                end: n.end,
                pitch: n.pitch,
                confidence: 1.,
            }
        })
        .collect()
}
fn synth_assets() -> BTreeMap<String, String> {
    muscriptor::assets()
        .into_iter()
        .map(|a| (a.filename, a.sha256))
        .collect()
}
fn synth_model_hash() -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&synth_assets()).expect("model metadata serializes"))
    )
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn same(a: &PitchNote, b: &PitchNote) -> bool {
    a.start == b.start && a.end == b.end && a.pitch == b.pitch && a.confidence == b.confidence
}
fn validate_piano_output(output: &piano_model::Output, duration: f64) -> Result<()> {
    ensure!(
        output.method == piano_model::METHOD
            && output.velocities.len() == output.notes.len()
            && output.velocities.iter().all(|v| (1..=127).contains(v)),
        "Piano multiview raw decoder identity/velocity mismatch"
    );
    ensure!(
        output.notes.iter().all(|n| n.start.is_finite()
            && n.start >= 0.
            && n.end.is_finite()
            && n.end > n.start
            && n.end <= duration + 1. / 16000.
            && (21..=108).contains(&n.pitch)
            && n.confidence.is_finite()
            && (0.0..=1.0).contains(&n.confidence)),
        "Invalid full raw Piano multiview output"
    );
    ensure!(
        output.pedals.iter().all(|p| p.start.is_finite()
            && p.start >= 0.
            && p.end.is_finite()
            && p.end > p.start
            && p.end <= duration + 1. / 16000.),
        "Invalid raw Piano multiview pedals"
    );
    Ok(())
}
fn parameters(sound: KeyboardSound) -> BTreeMap<String, f32> {
    match sound {
        KeyboardSound::Piano => BTreeMap::from([
            ("onset_threshold".into(), 0.3),
            ("offset_threshold".into(), 0.3),
            ("frame_threshold".into(), 0.1),
            ("pedal_offset_threshold".into(), 0.2),
        ]),
        KeyboardSound::Synth => BTreeMap::from([
            ("onset_threshold".into(), 0.5),
            ("frame_threshold".into(), 0.3),
        ]),
    }
}
pub fn implementation_hash(sound: KeyboardSound) -> String {
    let mut h = Sha256::new();
    let detector: &[u8] = match sound {
        KeyboardSound::Piano => include_bytes!("piano_model.rs"),
        KeyboardSound::Synth => include_bytes!("muscriptor.rs"),
    };
    for bytes in [
        include_bytes!("keyboard_detection.rs").as_slice(),
        detector,
        include_bytes!("models.rs").as_slice(),
        include_bytes!("keyboard_audio.rs").as_slice(),
    ] {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    format!("{:x}", h.finalize())
}
impl Evidence {
    /// Validate saved evidence without requiring the detector/model to be installed.
    /// Historical implementation hashes remain valid after application updates.
    pub fn validate(&self, transcription: &Transcription) -> Result<()> {
        ensure!(
            [1, 2, 3, 4, 5, 6].contains(&self.version),
            "Unsupported keyboard evidence version"
        );
        let manifest = transcription
            .keyboard_manifest()?
            .context("Keyboard detector evidence requires bound stems")?;
        manifest.validate_binding(&self.source_audio_sha256, self.duration_seconds)?;
        ensure!(
            (self.duration_seconds - manifest.frames as f64 / 44100.).abs() <= 1. / 44100.,
            "Keyboard evidence duration differs from the source sample clock"
        );
        ensure!(
            hash_valid(&self.implementation_sha256),
            "Invalid keyboard implementation hash"
        );
        if [2, 3].contains(&self.version) {
            ensure!(
                self.piano_audio.is_none() && self.piano_multiview.is_none(),
                "MuScriptor cannot claim a piano audio recipe"
            );
            return self.validate_muscriptor(transcription);
        }
        ensure!(
            self.muscriptor.is_none(),
            "Legacy detector evidence cannot contain MuScriptor output"
        );
        ensure!(
            self.version >= 5 || self.piano_multiview.is_none(),
            "Historical detector cannot claim multiview inference"
        );
        let (method, expected) = if self.version >= 4 {
            ensure!(
                self.sound == KeyboardSound::Piano,
                "v4 keyboard evidence requires Piano sound"
            );
            let receipt = self
                .piano_audio
                .as_ref()
                .context("Piano audio recipe is missing")?;
            receipt.validate(manifest)?;
            ensure!(
                receipt.source_audio_sha256 == self.source_audio_sha256,
                "Piano audio recipe belongs to another source"
            );
            let mut hashes = receipt.contributing_stem_sha256.clone();
            if self.version >= 5 {
                hashes.extend(stem_hashes(manifest, KeyboardSound::Piano)?);
            }
            (
                if self.version == 5 {
                    format!("{};{}", piano_method(), PIANO_SELECTION_V5)
                } else if self.version == 6 {
                    piano_multiview_method()
                } else {
                    piano_method()
                },
                hashes,
            )
        } else {
            ensure!(
                self.piano_audio.is_none(),
                "Legacy detector cannot claim a piano audio recipe"
            );
            (
                self.sound.method().to_owned(),
                stem_hashes(manifest, self.sound)?,
            )
        };
        ensure!(
            self.method == method
                && self.score_semantics == self.sound.score_semantics()
                && self.decoder_parameters == parameters(self.sound),
            "Keyboard decoder identity or score semantics changed"
        );
        let model = models::assets()
            .into_iter()
            .find(|a| a.id == self.sound.model_id())
            .context("Unknown keyboard model")?;
        ensure!(
            self.model_id == model.id && self.model_sha256 == model.sha256,
            "Keyboard model identity mismatch"
        );
        ensure!(
            self.stem_sha256 == expected,
            "Keyboard evidence stem binding mismatch"
        );
        if self.version >= 5 {
            let views = self
                .piano_multiview
                .as_ref()
                .context("Piano multiview raw outputs missing")?;
            for output in [&views.baseline, &views.residual] {
                validate_piano_output(output, self.duration_seconds)?;
            }
            ensure!(
                views.baseline.notes.iter().any(|n| n.confidence >= 0.65),
                "Piano recovery requires strong baseline evidence"
            );
            let (policy, indices) = if self.version == 5 {
                (
                    PIANO_SELECTION_V5,
                    piano_selected_indices_v5(&views.baseline.notes, &views.residual.notes),
                )
            } else {
                (
                    PIANO_SELECTION,
                    piano_selected_indices(&views.baseline.notes, &views.residual.notes),
                )
            };
            ensure!(
                views.selection_policy == policy && views.selected_indices == indices,
                "Piano multiview selection differs from frozen policy"
            );
            let (notes, velocities) = selected_piano(views);
            ensure!(
                notes.len() == self.raw_notes.len()
                    && notes.iter().zip(&self.raw_notes).all(|(a, b)| same(a, b))
                    && velocities == self.velocities,
                "Piano multiview notes/velocities differ from retained source indices"
            );
            ensure!(
                serde_json::to_value(&self.pedals)?
                    == serde_json::to_value(&views.baseline.pedals)?,
                "Piano multiview must preserve baseline pedal provenance"
            );
        }
        ensure!(
            self.raw_notes.iter().all(|n| n.start.is_finite()
                && n.start >= 0.
                && n.end.is_finite()
                && n.end > n.start
                && n.end
                    <= self.duration_seconds
                        + if self.version >= 4 {
                            1. / 16000.
                        } else {
                            1. / 22050.
                        }
                && n.pitch <= 127
                && n.confidence.is_finite()
                && (0.0..=1.0).contains(&n.confidence)),
            "Invalid keyboard raw candidate/source clock"
        );
        for notes in [&transcription.piano, &transcription.pro_piano] {
            ensure!(
                notes.is_empty()
                    || (notes.len() == self.raw_notes.len()
                        && notes.iter().zip(&self.raw_notes).all(|(a, b)| same(a, b))),
                "Keyboard arrangement input differs from raw detector notes"
            );
        }
        match self.sound {
            KeyboardSound::Piano => ensure!(
                self.basic_pitch_candidates.is_empty()
                    && self.velocities.len() == self.raw_notes.len()
                    && self.velocities.iter().all(|v| (1..=127).contains(v))
                    && self.raw_notes.iter().all(|n| (21..=108).contains(&n.pitch)),
                "Invalid piano velocity/pitch evidence"
            ),
            KeyboardSound::Synth => {
                ensure!(
                    self.velocities.is_empty()
                        && self.pedals.is_empty()
                        && self.basic_pitch_candidates.len() == self.raw_notes.len()
                        && self.basic_pitch_candidates.iter().zip(&self.raw_notes).all(
                            |(e, n)| same(&e.candidate, n)
                                && [e.onset_score, e.inferred_onset_score]
                                    .iter()
                                    .all(|s| s.is_finite() && (0.0..=1.0).contains(s))
                        ),
                    "Invalid synth decoder evidence"
                )
            }
        }
        ensure!(
            self.pedals.iter().all(|p| p.start.is_finite()
                && p.start >= 0.
                && p.end.is_finite()
                && p.end > p.start
                && p.end
                    <= self.duration_seconds + if self.version >= 4 { 1. / 16000. } else { 0. }),
            "Invalid separate piano pedal event"
        );
        Ok(())
    }
    fn validate_muscriptor(&self, transcription: &Transcription) -> Result<()> {
        let evidence = self
            .muscriptor
            .as_ref()
            .context("MuScriptor raw evidence is missing")?;
        let output = &evidence.output;
        ensure!(
            self.sound == KeyboardSound::Synth
                && self.model_id == "muscriptor-small"
                && self.model_sha256 == synth_model_hash()
                && output.model_assets == synth_assets(),
            "MuScriptor model identities differ"
        );
        ensure!(
            self.stem_sha256.is_empty()
                && self.basic_pitch_candidates.is_empty()
                && self.velocities.is_empty()
                && self.pedals.is_empty(),
            "MuScriptor full-mix evidence cannot claim stem inference, velocities or legacy scores"
        );
        ensure!(
            self.method == muscriptor::METHOD
                && output.method == muscriptor::METHOD
                && self.score_semantics == SYNTH_PRIORITY
                && self.decoder_parameters
                    == BTreeMap::from([("nominal_arrangement_priority".into(), 1.)]),
            "MuScriptor decoder/priority semantics differ"
        );
        ensure!(
            output.options.instrument_groups == [32]
                && output.options.force_ties
                && output.options.max_tokens == 2048,
            "MuScriptor advisory conditioning/options differ"
        );
        ensure!(
            output.duration_seconds.is_finite()
                && (output.duration_seconds - self.duration_seconds).abs() <= 1. / 16000.,
            "MuScriptor source duration differs"
        );
        ensure!(
            output.notes.iter().all(|n| n.start.is_finite()
                && n.start >= 0.
                && n.end.is_finite()
                && n.end > n.start
                && n.end <= output.duration_seconds
                && n.pitch <= 127
                && n.program <= 129
                && n.instrument == muscriptor::instrument_name(n.program)),
            "Invalid raw MuScriptor note/source clock/label"
        );
        let (policy, indices) = if self.version == 2 {
            (
                SYNTH_SELECTION_V2,
                legacy_synth_selected_indices(&output.notes),
            )
        } else {
            (SYNTH_SELECTION, synth_selected_indices(&output.notes))
        };
        ensure!(
            evidence.selection_policy == policy && evidence.selected_indices == indices,
            "MuScriptor selection indices/policy differ"
        );
        let selected = synth_notes(output, &evidence.selected_indices);
        ensure!(
            self.raw_notes.len() == selected.len()
                && self
                    .raw_notes
                    .iter()
                    .zip(&selected)
                    .all(|(a, b)| same(a, b)),
            "MuScriptor arrangement inputs/nominal priority differ from raw selection"
        );
        for notes in [&transcription.piano, &transcription.pro_piano] {
            ensure!(
                notes.is_empty()
                    || (notes.len() == selected.len()
                        && notes.iter().zip(&selected).all(|(a, b)| same(a, b))),
                "Keyboard arrangement input differs from selected MuScriptor notes"
            );
        }
        ensure!(
            output.chunks.len() == (output.duration_seconds / 5.).ceil() as usize,
            "MuScriptor chunk coverage differs"
        );
        for (i, chunk) in output.chunks.iter().enumerate() {
            ensure!(
                (chunk.start - i as f64 * 5.).abs() < 1e-9
                    && (chunk.end - (chunk.start + 5.).min(output.duration_seconds)).abs() < 1e-9
                    && chunk.instrument_ids == [34]
                    && chunk.generated_tokens.len() <= 2048
                    && chunk
                        .generated_tokens
                        .iter()
                        .chain(&chunk.forced_tokens)
                        .all(|t| (0..muscriptor::VOCAB_SIZE as i64).contains(t))
                    && chunk.elapsed_seconds.is_finite()
                    && chunk.elapsed_seconds >= 0.
                    && chunk.reached_token_limit != chunk.emitted_eos,
                "Invalid MuScriptor chunk/token/conditioning receipt"
            );
            ensure!(
                if chunk.emitted_eos {
                    chunk.generated_tokens.last() == Some(&muscriptor::EOS)
                        && !chunk.generated_tokens[..chunk.generated_tokens.len() - 1]
                            .contains(&muscriptor::EOS)
                } else {
                    chunk.generated_tokens.len() == 2048
                        && !chunk.generated_tokens.contains(&muscriptor::EOS)
                },
                "MuScriptor EOS/token-limit flags differ from tokens"
            );
        }
        // Decode retained tokens again: labels, releases and diagnostic counters
        // cannot be silently substituted while preserving the raw token receipt.
        let mut decoder = muscriptor::NoteDecoder::new();
        for (index, chunk) in output.chunks.iter().enumerate() {
            decoder.begin_chunk(chunk.start, chunk.end)?;
            ensure!(
                chunk.forced_tokens
                    == if index == 0 {
                        vec![]
                    } else {
                        decoder.tie_tokens()
                    },
                "MuScriptor forced tie prelude differs from decoder state"
            );
            for &token in chunk.forced_tokens.iter().chain(
                chunk
                    .generated_tokens
                    .iter()
                    .take_while(|t| **t != muscriptor::EOS),
            ) {
                decoder.feed(token);
            }
        }
        let (notes, diagnostics) = decoder.finish(output.duration_seconds);
        ensure!(
            serde_json::to_value(&notes)? == serde_json::to_value(&output.notes)?
                && serde_json::to_value(diagnostics)? == serde_json::to_value(&output.diagnostics)?,
            "MuScriptor raw notes/diagnostics differ from retained tokens"
        );
        Ok(())
    }
}
fn stem_hashes(manifest: &StemManifest, sound: KeyboardSound) -> Result<BTreeMap<String, String>> {
    let ids: &[StemId] = match sound {
        KeyboardSound::Piano => &[StemId::Piano],
        KeyboardSound::Synth => &[StemId::Piano, StemId::Other],
    };
    ids.iter()
        .map(|id| {
            let stem = manifest
                .stems
                .iter()
                .find(|s| s.id == *id)
                .with_context(|| {
                    format!(
                        "{} sound requires a source-bound {} stem",
                        if sound == KeyboardSound::Piano {
                            "Piano"
                        } else {
                            "Synth"
                        },
                        id.as_str()
                    )
                })?;
            Ok((id.as_str().into(), stem.sha256.clone()))
        })
        .collect()
}
#[cfg(not(target_arch = "wasm32"))]
pub fn detect(
    audio: &Path,
    manifest: &StemManifest,
    model_dir: &Path,
    sound: KeyboardSound,
) -> Result<Evidence> {
    manifest.validate_binding(
        &manifest.source_audio_sha256,
        manifest.frames as f64 / 44100.,
    )?;
    ensure!(
        models::hash_file(audio)? == manifest.source_audio_sha256,
        "Keyboard source differs from bound recording"
    );
    if sound == KeyboardSound::Synth {
        for id in ["muscriptor-cond", "muscriptor-lm"] {
            models::verified(model_dir, id)?;
        }
        let output = muscriptor::transcribe(
            audio,
            model_dir,
            muscriptor::Options {
                instrument_groups: vec![32],
                ..Default::default()
            },
        )?;
        ensure!(
            models::hash_file(audio)? == manifest.source_audio_sha256,
            "MuScriptor source changed during inference"
        );
        for id in ["muscriptor-cond", "muscriptor-lm"] {
            models::verified(model_dir, id)?;
        }
        let indices = synth_selected_indices(&output.notes);
        return Ok(Evidence {
            version: 3,
            sound,
            source_audio_sha256: manifest.source_audio_sha256.clone(),
            duration_seconds: manifest.frames as f64 / 44100.,
            stem_sha256: BTreeMap::new(),
            model_id: "muscriptor-small".into(),
            model_sha256: synth_model_hash(),
            implementation_sha256: implementation_hash(sound),
            method: muscriptor::METHOD.into(),
            score_semantics: SYNTH_PRIORITY.into(),
            decoder_parameters: BTreeMap::from([("nominal_arrangement_priority".into(), 1.)]),
            raw_notes: synth_notes(&output, &indices),
            basic_pitch_candidates: vec![],
            velocities: vec![],
            pedals: vec![],
            muscriptor: Some(SynthEvidence {
                output,
                selection_policy: SYNTH_SELECTION.into(),
                selected_indices: indices,
            }),
            piano_audio: None,
            piano_multiview: None,
        });
    }
    let model = models::verified(model_dir, sound.model_id())?;
    let mut baseline_manifest = manifest.clone();
    baseline_manifest.stems.retain(|s| s.id == StemId::Piano);
    drop(baseline_manifest.load(audio)?);
    let mut transcriber = piano_model::Transcriber::load(&model)?;
    let baseline = transcriber.transcribe(&baseline_manifest.stems[0].path, Default::default())?;
    let (version, method, hashes, output, piano_audio, piano_multiview) = if baseline
        .notes
        .iter()
        .any(|n| n.confidence >= 0.65)
    {
        let prepared = keyboard_audio::prepare(audio, manifest)?;
        let residual = piano_model::decode(
            &transcriber.infer_samples(&prepared.samples)?,
            Default::default(),
        )?;
        let indices = piano_selected_indices(&baseline.notes, &residual.notes);
        let pedals = baseline.pedals.clone();
        let views = PianoViews {
            baseline,
            residual,
            selection_policy: PIANO_SELECTION.into(),
            selected_indices: indices,
        };
        let (notes, velocities) = selected_piano(&views);
        let output = piano_model::Output {
            method: piano_model::METHOD.into(),
            notes,
            velocities,
            pedals,
        };
        let mut hashes = prepared.receipt.contributing_stem_sha256.clone();
        hashes.extend(stem_hashes(manifest, sound)?);
        (
            6,
            piano_multiview_method(),
            hashes,
            output,
            Some(prepared.receipt),
            Some(views),
        )
    } else {
        eprintln!(
            "Piano review: no strong baseline piano attacks; skipped harmonic residual recovery to avoid unsupported accompaniment. Existing baseline output retained."
        );
        (
            1,
            sound.method().into(),
            stem_hashes(manifest, sound)?,
            baseline,
            None,
            None,
        )
    };
    ensure!(
        models::hash_file(&model)?
            == models::assets()
                .into_iter()
                .find(|a| a.id == sound.model_id())
                .unwrap()
                .sha256,
        "Keyboard model changed during inference"
    );
    for (name, hash) in &hashes {
        let stem = manifest
            .stems
            .iter()
            .find(|s| s.id.as_str() == name)
            .unwrap();
        ensure!(
            models::hash_file(&stem.path)? == *hash,
            "Keyboard stem changed during inference"
        );
    }
    ensure!(
        models::hash_file(audio)? == manifest.source_audio_sha256,
        "Keyboard source changed during inference"
    );
    Ok(Evidence {
        version,
        sound,
        source_audio_sha256: manifest.source_audio_sha256.clone(),
        duration_seconds: manifest.frames as f64 / 44100.,
        stem_sha256: hashes,
        model_id: sound.model_id().into(),
        model_sha256: models::hash_file(&model)?,
        implementation_sha256: implementation_hash(sound),
        method,
        score_semantics: sound.score_semantics().into(),
        decoder_parameters: parameters(sound),
        raw_notes: output.notes,
        basic_pitch_candidates: vec![],
        velocities: output.velocities,
        pedals: output.pedals,
        muscriptor: None,
        piano_audio,
        piano_multiview,
    })
}
/// Model ids needed by a complete Auto-Chart job, excluding optional lyric ASR.
#[cfg(not(target_arch = "wasm32"))]
pub fn required_models(
    instruments: &[String],
    sound: KeyboardSound,
    full_mix_drums: bool,
) -> Vec<&'static str> {
    required_models_with_quality(instruments, sound, PianoQuality::Standard, full_mix_drums)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn required_models_with_quality(
    instruments: &[String],
    sound: KeyboardSound,
    quality: PianoQuality,
    full_mix_drums: bool,
) -> Vec<&'static str> {
    let mut ids = vec![];
    if instruments.iter().any(|s| s == "drums") {
        ids.push("drums");
    }
    if instruments
        .iter()
        .any(|s| matches!(s.as_str(), "guitar" | "bass" | "vocals"))
    {
        ids.push("pitch");
    }
    if instruments
        .iter()
        .any(|s| matches!(s.as_str(), "piano" | "pro_piano"))
    {
        match sound {
            KeyboardSound::Piano => ids.push("piano"),
            KeyboardSound::Synth => ids.extend(["muscriptor-cond", "muscriptor-lm"]),
        }
    }
    let hq = high_quality_piano(instruments, sound, quality);
    if hq {
        ids.push(crate::keyboard_separation::MODEL_ID);
    }
    if !full_mix_drums
        && (!hq
            || instruments
                .iter()
                .any(|s| !matches!(s.as_str(), "piano" | "pro_piano")))
    {
        ids.push("separation");
    }
    ids
}
