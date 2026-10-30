//! Rust ports of HeroSaver's voice-layer reduction and chartgen's stable
//! pitch/register map. See THIRD_PARTY.md for pinned sources and changes.
use crate::{
    chart::{Note, Tracks},
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum GuitarSource {
    #[default]
    Guitar,
    Other,
    Piano,
}
impl GuitarSource {
    pub fn audition_stem(self, track: &str, vocals: bool) -> crate::stem_review::StemId {
        use crate::stem_review::StemId;
        if track == "Vocals" || vocals {
            StemId::Vocals
        } else if crate::keys::is_keys(track) {
            StemId::Piano
        } else if track.ends_with("Drums") {
            StemId::Drums
        } else if track.ends_with("DoubleBass") {
            StemId::Bass
        } else {
            self.stem()
        }
    }
    pub fn stem(self) -> crate::stem_review::StemId {
        match self {
            Self::Guitar => crate::stem_review::StemId::Guitar,
            Self::Other => crate::stem_review::StemId::Other,
            Self::Piano => crate::stem_review::StemId::Piano,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Voice {
    Kick,
    Snare,
    HiHat,
    HighTom,
    MidTom,
    LowTom,
    Crash,
    Ride,
    Tom,
    Cymbal,
}

impl Voice {
    pub fn lane(self) -> u8 {
        match self {
            Self::Kick => 0,
            Self::Snare => 1,
            Self::HiHat | Self::HighTom => 2,
            Self::Ride | Self::MidTom | Self::Tom => 3,
            Self::Crash | Self::LowTom | Self::Cymbal => 4,
        }
    }
    pub fn cymbal(self) -> bool {
        matches!(self, Self::HiHat | Self::Crash | Self::Ride | Self::Cymbal)
    }
    pub fn from_gm(pitch: u8) -> Option<Self> {
        Some(match pitch {
            35 | 36 => Self::Kick,
            37 | 38 | 40 => Self::Snare,
            42 | 44 | 46 => Self::HiHat,
            48 | 50 => Self::HighTom,
            45 | 47 => Self::MidTom,
            41 | 43 => Self::LowTom,
            49 | 52 | 55 | 57 => Self::Crash,
            51 | 53 | 59 => Self::Ride,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrumHit {
    pub time: f64,
    pub voice: Voice,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchNote {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PitchCandidateSource {
    NetworkOnset,
    InferredFrameRise,
    FrameRecovery,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchCandidateEvidence {
    pub candidate: PitchNote,
    pub source: PitchCandidateSource,
    pub onset_score: f32,
    pub inferred_onset_score: f32,
}

/// Guitar-only source verification; scores are evidence, not calibrated odds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuitarPolyphonicNote {
    pub evidence: PitchCandidateEvidence,
    pub original_rejection: String,
    pub source_attack_seconds: f64,
    pub window_start: f64,
    pub window_end: f64,
    pub minimum_fundamental_power: f64,
    pub minimum_target_variance_gain: f64,
    pub minimum_source_explained: f64,
    pub maximum_subharmonic_power: f64,
    pub frequency_offset_hz: f64,
    pub preexisting_power_fraction: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortPitchCandidate {
    pub evidence: PitchCandidateEvidence,
    pub retained: bool,
    pub reason: String,
    pub source_attack_seconds: Option<f64>,
    pub periodic_pitch: Option<f64>,
    pub periodicity: Option<f64>,
    pub fundamental_fraction: Option<f64>,
    pub window_start: Option<f64>,
    pub window_end: Option<f64>,
}

/// Compatibility name retained for saved bass evidence and existing callers.
pub type BassShortCandidate = ShortPitchCandidate;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BassCorrection {
    pub original: PitchNote,
    pub revised_pitch: u8,
    pub periodic_pitch: f64,
    pub clarity: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BassTimingCorrection {
    pub original: PitchNote,
    pub revised_start: f64,
    pub before_rms: f64,
    pub after_rms: f64,
    pub periodic_pitch: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BassFundamentalSupport {
    pub early_explained: f64,
    pub later_explained: f64,
    pub added_variance_fraction: f64,
    pub fundamental_fraction: f64,
    pub phase_error_radians: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BassShadowMethod {
    StrongProjection,
    JointPhase,
    JointTransient,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BassShadowSupport {
    pub earlier: PitchNote,
    pub later: PitchNote,
    pub early_fundamental_fraction: f64,
    pub method: BassShadowMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joint_fit: Option<BassFundamentalSupport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BassAttackReview {
    pub candidate: PitchNote,
    pub nearest_attack_seconds: Option<f64>,
    pub residual_seconds: Option<f64>,
    pub supported: bool,
}

/// Overlapping harmonic-family candidates that refinement could not reconcile.
/// Both may represent real playing; this receipt is a review prompt, not a fix.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BassHarmonicConflict {
    pub candidate: PitchNote,
    pub competing_candidate: PitchNote,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transcription {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bass_decoder: Option<crate::bass_analysis::Decoder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bass_analysis: Option<crate::bass_analysis::Analysis>,
    /// Optional source/hash-bound isolation provenance. Legacy hand-authored or
    /// model-free events remain valid without a separator or external cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<crate::stem_review::StemManifest>,
    /// Authoritative Piano analysis stems when a mixed job keeps its existing
    /// separator for the other instruments. This is never a hybrid manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_analysis: Option<crate::stem_review::StemManifest>,
    /// Raw keyboard detections and source/model/score provenance, before arrangement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_detection: Option<crate::keyboard_detection::Evidence>,
    #[serde(default)]
    pub drums: Vec<DrumHit>,
    /// Additional model peaks corroborated by attacks in the analyzed audio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drum_recovered_hits: Vec<DrumHit>,
    /// Source-only kit proposals; raw five-class hits above remain unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drum_articulations: Vec<crate::drum_articulation::Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drum_articulation_source: Option<crate::drum_articulation::Source>,
    #[serde(default)]
    pub guitar: Vec<PitchNote>,
    /// Explicit audio role arranged into five-fret Guitar; not inferred identity.
    #[serde(default)]
    pub guitar_source: GuitarSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guitar_candidates: Vec<PitchCandidateEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guitar_short_candidates: Vec<ShortPitchCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guitar_polyphonic_notes: Vec<GuitarPolyphonicNote>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub guitar_polyphonic_diagnostics: BTreeMap<String, usize>,
    #[serde(default)]
    pub bass: Vec<PitchNote>,
    /// Piano-source candidates arranged as five-lane Keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub piano: Vec<PitchNote>,
    /// Piano-source candidates arranged with their actual MIDI pitches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pro_piano: Vec<PitchNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_corrections: Vec<BassCorrection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_timing_corrections: Vec<BassTimingCorrection>,
    /// Original decoder evidence, before audio refinement changes pitch/time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_candidates: Vec<PitchCandidateEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_short_candidates: Vec<BassShortCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_shadow_support: Vec<BassShadowSupport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_attack_reviews: Vec<BassAttackReview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bass_harmonic_conflicts: Vec<BassHarmonicConflict>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Transcription {
    pub fn keyboard_manifest(&self) -> Result<Option<&crate::stem_review::StemManifest>> {
        if let Some(analysis) = &self.keyboard_analysis {
            let primary = self
                .isolation
                .as_ref()
                .context("Piano analysis requires primary source isolation")?;
            primary
                .validate_binding(&primary.source_audio_sha256, primary.frames as f64 / 44100.)?;
            ensure!(
                self.keyboard_detection
                    .as_ref()
                    .is_some_and(|e| e.sound == crate::keyboard_detection::KeyboardSound::Piano),
                "Separate keyboard analysis requires Piano detector evidence"
            );
            ensure!(
                analysis.frames == primary.frames
                    && analysis
                        .source_audio_sha256
                        .eq_ignore_ascii_case(&primary.source_audio_sha256),
                "Piano analysis and primary stems describe different source clocks"
            );
            analysis
                .validate_binding(&primary.source_audio_sha256, primary.frames as f64 / 44100.)?;
            ensure!(
                analysis.ids() == crate::stem_review::StemId::ALL.into_iter().collect(),
                "Piano analysis must retain its complete six-stem source manifest"
            );
            Ok(Some(analysis))
        } else {
            Ok(self.isolation.as_ref())
        }
    }
    pub fn validate(&self) -> Result<()> {
        crate::drum_articulation::validate(&self.drum_articulations, &self.drums)?;
        crate::drum_articulation::validate_source(
            self.drum_articulation_source.as_ref(),
            &self.drum_articulations,
            self.isolation.as_ref(),
        )?;
        self.keyboard_manifest()?;
        if let Some(evidence) = &self.keyboard_detection {
            evidence.validate(self)?;
        }
        crate::bass_analysis::validate(self)?;
        if let Some(isolation) = &self.isolation {
            isolation.validate_binding(
                &isolation.source_audio_sha256,
                isolation.frames as f64 / isolation.sample_rate as f64,
            )?;
        }
        ensure!(
            self.drums
                .iter()
                .chain(&self.drum_recovered_hits)
                .all(|h| h.time.is_finite() && h.time >= 0.0 && valid_conf(h.confidence)),
            "Invalid drum event"
        );
        ensure!(
            self.drum_recovered_hits.iter().all(|hit| self
                .drums
                .iter()
                .any(|raw| raw.time == hit.time
                    && raw.voice == hit.voice
                    && raw.confidence == hit.confidence)),
            "Recovered drum hit is absent from transcription"
        );
        ensure!(
            self.guitar
                .iter()
                .chain(&self.bass)
                .chain(&self.piano)
                .chain(&self.pro_piano)
                .all(|n| n.start.is_finite()
                    && n.start >= 0.0
                    && n.end.is_finite()
                    && n.end > n.start
                    && n.pitch <= 127
                    && valid_conf(n.confidence)),
            "Invalid pitched note"
        );
        ensure!(
            self.bass_timing_corrections
                .iter()
                .all(|correction| correction.original.start.is_finite()
                    && correction.original.start >= 0.
                    && correction.original.end.is_finite()
                    && correction.original.end > correction.original.start
                    && correction.revised_start.is_finite()
                    && correction.revised_start >= 0.
                    && correction.revised_start < correction.original.end
                    && correction.before_rms.is_finite()
                    && correction.before_rms >= 0.
                    && correction.after_rms.is_finite()
                    && correction.after_rms > 0.
                    && correction.periodic_pitch.is_finite()),
            "Invalid bass timing correction"
        );
        ensure!(
            self.bass_candidates
                .iter()
                .chain(&self.guitar_candidates)
                .chain(
                    self.guitar_short_candidates
                        .iter()
                        .map(|candidate| &candidate.evidence)
                )
                .chain(
                    self.bass_short_candidates
                        .iter()
                        .map(|candidate| &candidate.evidence)
                )
                .all(|evidence| evidence.candidate.start.is_finite()
                    && evidence.candidate.start >= 0.
                    && evidence.candidate.end.is_finite()
                    && evidence.candidate.end > evidence.candidate.start
                    && evidence.candidate.pitch <= 127
                    && valid_conf(evidence.candidate.confidence)
                    && valid_conf(evidence.onset_score)
                    && valid_conf(evidence.inferred_onset_score)),
            "Invalid pitch candidate evidence"
        );
        ensure!(
            self.guitar_polyphonic_notes.iter().all(|r| {
                let e = &r.evidence;
                let n = &e.candidate;
                let nominal = 440. * 2f64.powf((n.pitch as f64 - 69.) / 12.);
                self.guitar_source == GuitarSource::Guitar
                    && valid_pitch_note(n)
                    && (40..=88).contains(&n.pitch)
                    && n.confidence >= 0.3
                    && e.source == PitchCandidateSource::NetworkOnset
                    && valid_conf(e.onset_score)
                    && e.onset_score >= 0.5
                    && valid_conf(e.inferred_onset_score)
                    && n.end - n.start >= 0.055
                    && matches!(
                        r.original_rejection.as_str(),
                        "network_support_not_strong"
                            | "unstable_source_pitch"
                            | "source_pitch_disagrees"
                    )
                    && [
                        r.source_attack_seconds,
                        r.window_start,
                        r.window_end,
                        r.minimum_fundamental_power,
                        r.minimum_target_variance_gain,
                        r.minimum_source_explained,
                        r.maximum_subharmonic_power,
                        r.frequency_offset_hz,
                    ]
                    .iter()
                    .all(|v| v.is_finite())
                    && r.source_attack_seconds >= 0.
                    && (r.source_attack_seconds - n.start).abs() <= 0.035
                    && r.window_start >= n.start
                    && r.window_end <= n.end
                    && r.window_end - r.window_start >= 0.045
                    && (r.window_end - r.window_start) * nominal >= 3.5
                    && r.minimum_fundamental_power >= 0.000064
                    && (0.1..=1.).contains(&r.minimum_target_variance_gain)
                    && (0. ..=1.).contains(&r.minimum_source_explained)
                    && r.maximum_subharmonic_power >= 0.
                    && r.maximum_subharmonic_power * 4. <= r.minimum_fundamental_power
                    && r.frequency_offset_hz.abs() <= nominal * 0.021
                    && r.preexisting_power_fraction.is_none_or(|fraction| {
                        fraction.is_finite() && (0. ..0.25).contains(&fraction)
                    })
                    && self.guitar.iter().any(|actual| {
                        actual.start == n.start
                            && actual.end == n.end
                            && actual.pitch == n.pitch
                            && actual.confidence == n.confidence
                    })
                    && self.guitar_candidates.iter().any(|actual| {
                        actual.candidate.start == n.start
                            && actual.candidate.end == n.end
                            && actual.candidate.pitch == n.pitch
                            && actual.candidate.confidence == n.confidence
                            && actual.source == e.source
                            && actual.onset_score == e.onset_score
                            && actual.inferred_onset_score == e.inferred_onset_score
                    })
                    && self.guitar_short_candidates.iter().any(|raw| {
                        !raw.retained
                            && raw.reason == r.original_rejection
                            && raw.evidence.candidate.start == n.start
                            && raw.evidence.candidate.end == n.end
                            && raw.evidence.candidate.pitch == n.pitch
                            && raw.evidence.candidate.confidence == n.confidence
                            && raw.evidence.source == e.source
                            && raw.evidence.onset_score == e.onset_score
                            && raw.evidence.inferred_onset_score == e.inferred_onset_score
                    })
            }),
            "Invalid polyphonic guitar recovery evidence"
        );
        ensure!(
            self.bass_short_candidates
                .iter()
                .map(|candidate| (candidate, &self.bass_candidates))
                .chain(
                    self.guitar_short_candidates
                        .iter()
                        .map(|candidate| (candidate, &self.guitar_candidates))
                )
                .all(
                    |(candidate, raw_candidates)| !candidate.reason.trim().is_empty()
                        && candidate
                            .source_attack_seconds
                            .is_none_or(|value| value.is_finite() && value >= 0.)
                        && candidate
                            .periodic_pitch
                            .is_none_or(|value| value.is_finite() && (0. ..=127.).contains(&value))
                        && candidate
                            .periodicity
                            .is_none_or(|value| value.is_finite() && (-1. ..=1.).contains(&value))
                        && candidate
                            .fundamental_fraction
                            .is_none_or(|value| value.is_finite() && (0. ..=1.).contains(&value))
                        && match (candidate.window_start, candidate.window_end) {
                            (Some(start), Some(end)) =>
                                start.is_finite() && end.is_finite() && start >= 0. && end > start,
                            (None, None) => true,
                            _ => false,
                        }
                        && (!candidate.retained
                            || (candidate.source_attack_seconds.is_some()
                                && candidate.periodic_pitch.is_some()
                                && candidate.periodicity.is_some()
                                && candidate.fundamental_fraction.is_some()
                                && candidate
                                    .window_start
                                    .zip(candidate.window_end)
                                    .is_some_and(|(start, end)| start
                                        >= candidate.evidence.candidate.start
                                        && end <= candidate.evidence.candidate.end)
                                && raw_candidates.iter().any(|raw| {
                                    let evidence = &candidate.evidence;
                                    raw.source == evidence.source
                                        && raw.onset_score == evidence.onset_score
                                        && raw.inferred_onset_score == evidence.inferred_onset_score
                                        && raw.candidate.start == evidence.candidate.start
                                        && raw.candidate.end == evidence.candidate.end
                                        && raw.candidate.pitch == evidence.candidate.pitch
                                        && raw.candidate.confidence == evidence.candidate.confidence
                                })))
                ),
            "Invalid short pitch candidate support"
        );
        ensure!(
            self.bass_shadow_support
                .iter()
                .all(|support| valid_pitch_note(&support.earlier)
                    && valid_pitch_note(&support.later)
                    && support.early_fundamental_fraction.is_finite()
                    && support.early_fundamental_fraction >= 0.
                    && support.joint_fit.as_ref().is_none_or(|fit| [
                        fit.early_explained,
                        fit.later_explained,
                        fit.added_variance_fraction,
                        fit.fundamental_fraction,
                        fit.phase_error_radians
                    ]
                    .iter()
                    .all(|value| value.is_finite()))),
            "Invalid bass harmonic support"
        );
        ensure!(
            self.bass_attack_reviews
                .iter()
                .all(|review| valid_pitch_note(&review.candidate)
                    && review
                        .nearest_attack_seconds
                        .is_none_or(|time| time.is_finite() && time >= 0.)
                    && review.residual_seconds.is_none_or(f64::is_finite)
                    && review.supported
                        == review
                            .residual_seconds
                            .is_some_and(|residual| residual.abs() <= 0.050)),
            "Invalid bass attack review"
        );
        ensure!(
            self.bass_harmonic_conflicts
                .iter()
                .all(|conflict| valid_pitch_note(&conflict.candidate)
                    && valid_pitch_note(&conflict.competing_candidate)
                    && crate::refine::is_close_harmonic_pair(
                        &conflict.candidate,
                        &conflict.competing_candidate
                    )),
            "Invalid bass harmonic conflict"
        );
        Ok(())
    }
}

fn valid_conf(c: f32) -> bool {
    c.is_finite() && (0.0..=1.0).contains(&c)
}

fn valid_pitch_note(note: &PitchNote) -> bool {
    note.start.is_finite()
        && note.start >= 0.
        && note.end.is_finite()
        && note.end > note.start
        && note.pitch <= 127
        && valid_conf(note.confidence)
}

pub fn tracks(input: &Transcription, timing: &TempoMap, pro: bool) -> Result<Tracks> {
    input.validate()?;
    let mut out = Tracks::new();
    let pro_keys = crate::pro_keys::arrange_transcription(input, timing)?;
    for (index, difficulty) in ["Easy", "Medium", "Hard", "Expert"].iter().enumerate() {
        for (suffix, notes, pro_keys) in [("Keyboard", &input.piano, false)] {
            if !notes.is_empty() {
                let arranged = crate::keys::arrange(notes, timing, index, pro_keys)?;
                if !arranged.is_empty() {
                    out.insert(format!("{difficulty}{suffix}"), arranged);
                }
            }
        }
        if !pro_keys.notes.is_empty() {
            out.insert(
                format!("{difficulty}ProKeys"),
                crate::pro_keys::reduce(&pro_keys, timing, index),
            );
        }
        if !input.drums.is_empty() {
            out.insert(
                format!("{difficulty}Drums"),
                if index == 3 && !input.drum_articulations.is_empty() {
                    articulated_drums(&input.drums, &input.drum_articulations, timing, pro)
                } else {
                    drums(&input.drums, timing, index, pro)
                },
            );
        }
        if !input.guitar.is_empty() {
            out.insert(
                format!("{difficulty}Single"),
                pitched(&input.guitar, timing, index, false),
            );
        }
        if !input.bass.is_empty() {
            out.insert(
                format!("{difficulty}DoubleBass"),
                pitched(&input.bass, timing, index, true),
            );
        }
    }
    Ok(out)
}

pub fn drum_tier_allows(voice: Voice, tier: usize) -> bool {
    !(tier == 0 && !matches!(voice, Voice::Kick | Voice::Snare | Voice::HiHat)
        || tier == 1
            && matches!(
                voice,
                Voice::HighTom | Voice::Crash | Voice::Ride | Voice::Cymbal
            ))
}

/// HeroSaver layer slots shared with the existing autochart mapper.
pub fn drum_layer_grid(voice: Voice, tier: usize, resolution: u32) -> Option<(u8, u32)> {
    let grid = match voice {
        Voice::Kick if tier == 0 => resolution,
        Voice::Kick if tier == 1 => resolution / 2,
        v if v.cymbal() && tier < 2 => resolution,
        v if v.cymbal() && tier == 2 => resolution / 4,
        Voice::Tom | Voice::HighTom | Voice::MidTom | Voice::LowTom if tier == 1 => resolution,
        _ => 0,
    };
    let layer = if voice == Voice::Kick {
        0
    } else if voice.cymbal() {
        1
    } else {
        2
    };
    (grid > 0).then_some((layer, grid))
}

fn articulated_drums(
    hits: &[DrumHit],
    evidence: &[crate::drum_articulation::Evidence],
    timing: &TempoMap,
    pro: bool,
) -> Vec<Note> {
    drums(
        &articulation_projection(hits, evidence, timing, pro),
        timing,
        3,
        pro,
    )
}

fn articulation_projection(
    hits: &[DrumHit],
    evidence: &[crate::drum_articulation::Evidence],
    timing: &TempoMap,
    pro: bool,
) -> Vec<DrumHit> {
    let mut original_selected = vec![];
    let baseline = drums_traced(hits, timing, 3, pro, Some(&mut original_selected));
    original_selected.sort_unstable();
    let counts = |notes: &[Note]| {
        let mut result = BTreeMap::new();
        for note in notes.iter().filter(|n| n.lane <= 4) {
            *result.entry(note.tick).or_insert(0usize) += 1;
        }
        result
    };
    let original = counts(&baseline);
    let mut proposed = crate::drum_articulation::proposed_hits(hits, evidence);
    let mut revised_selected = vec![];
    let revised = drums_traced(&proposed, timing, 3, pro, Some(&mut revised_selected));
    revised_selected.sort_unstable();
    let after = counts(&revised);
    if original == after && original_selected == revised_selected {
        return proposed;
    }
    // A relocation must not merge two hands into one pad or change which onset
    // survives a collision. Revert the affected group, then verify again.
    let mut affected: Vec<_> = original
        .keys()
        .chain(after.keys())
        .copied()
        .filter(|t| original.get(t) != after.get(t))
        .collect();
    affected.extend(
        original_selected
            .iter()
            .chain(&revised_selected)
            .filter(|entry| original_selected.contains(entry) != revised_selected.contains(entry))
            .map(|(_, tick)| *tick),
    );
    for (raw, hit) in hits.iter().zip(&mut proposed) {
        if affected
            .iter()
            .any(|tick| (timing.seconds(*tick) - raw.time).abs() <= 0.021)
        {
            hit.voice = raw.voice;
        }
    }
    revised_selected.clear();
    let revised = drums_traced(&proposed, timing, 3, pro, Some(&mut revised_selected));
    revised_selected.sort_unstable();
    if counts(&revised) == original && revised_selected == original_selected {
        proposed
    } else {
        hits.to_vec()
    }
}

fn drums(hits: &[DrumHit], timing: &TempoMap, tier: usize, pro: bool) -> Vec<Note> {
    drums_traced(hits, timing, tier, pro, None)
}

/// Actual selected raw indices/group ticks/voices after hand caps and collision
/// fallback. Diagnostic proposals for discarded hypotheses are not played gems.
pub fn played_drum_articulations(
    input: &Transcription,
    timing: &TempoMap,
) -> Vec<(usize, u64, Voice)> {
    let projected = articulation_projection(&input.drums, &input.drum_articulations, timing, false);
    let mut selected = vec![];
    drums_traced(&projected, timing, 3, false, Some(&mut selected));
    selected
        .into_iter()
        .map(|(index, tick)| (index, tick, projected[index].voice))
        .collect()
}

fn drums_traced(
    hits: &[DrumHit],
    timing: &TempoMap,
    tier: usize,
    pro: bool,
    mut selected: Option<&mut Vec<(usize, u64)>>,
) -> Vec<Note> {
    let mut candidates: Vec<_> = hits
        .iter()
        .filter_map(|h| timing.tick(h.time).map(|t| (t, h)))
        .collect();
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.confidence.total_cmp(&a.1.confidence))
    });
    let mut slots = BTreeSet::new();
    let mut by_tick: BTreeMap<u64, BTreeMap<u8, &DrumHit>> = BTreeMap::new();
    for (tick, hit) in candidates {
        if !drum_tier_allows(hit.voice, tier) {
            continue;
        }
        // HeroSaver's layer reduction: one timekeeping/kick/fill hit per slot.
        if let Some((layer, grid)) = drum_layer_grid(hit.voice, tier, timing.resolution) {
            let slot = (tick as f64 / grid as f64).round() as u64;
            if !slots.insert((layer, slot)) {
                continue;
            }
        }
        let lane = hit.voice.lane();
        // A model's independent class peaks can differ by one or two frames
        // for a single physical attack. Apply the hand limit across 20 ms,
        // rather than allowing a third hand just because the tick differs.
        let group = by_tick
            .range(..=tick)
            .next_back()
            .map(|(t, _)| *t)
            .filter(|t| timing.seconds(tick) - timing.seconds(*t) <= 0.020 + 1e-9)
            .unwrap_or(tick);
        let existing = by_tick.entry(group).or_default().entry(lane).or_insert(hit);
        if hit.confidence > existing.confidence {
            *existing = hit;
        }
    }
    let mut out = Vec::new();
    for (tick, voices) in by_tick {
        let mut pads: Vec<_> = voices.iter().filter(|(lane, _)| **lane != 0).collect();
        // At most two hand strikes plus a kick; prioritize snare, then toms,
        // then confidence. Deterministic ties use lane order.
        pads.sort_by(|(la, a), (lb, b)| {
            let priority = |h: &DrumHit| {
                if h.voice == Voice::Snare {
                    0
                } else if !h.voice.cymbal() {
                    1
                } else {
                    2
                }
            };
            priority(a)
                .cmp(&priority(b))
                .then(b.confidence.total_cmp(&a.confidence))
                .then(la.cmp(lb))
        });
        let has_snare = voices.contains_key(&1);
        if voices.contains_key(&0) && !(tier < 2 && has_snare) {
            out.push(Note {
                tick,
                lane: 0,
                sustain: 0,
            });
        }
        for (lane, hit) in pads.into_iter().take(2) {
            if let Some(selected) = selected.as_deref_mut()
                && let Some(index) = hits.iter().position(|raw| std::ptr::eq(raw, *hit))
            {
                selected.push((index, tick));
            }
            out.push(Note {
                tick,
                lane: *lane,
                sustain: 0,
            });
            if pro && hit.voice.cymbal() {
                out.push(Note {
                    tick,
                    lane: 64 + *lane,
                    sustain: 0,
                });
            }
        }
    }
    out.sort();
    out
}

fn percentile(values: &[u8], quantile: f64) -> f64 {
    let index = quantile * (values.len() - 1) as f64;
    let lo = index.floor() as usize;
    let hi = index.ceil() as usize;
    values[lo] as f64 + (values[hi] as f64 - values[lo] as f64) * index.fract()
}

/// Shared deterministic tier policy for audio mapping and Expert-linked edits.
/// Input fret lanes remain identities; this fixed compression does not infer
/// pitches or recalculate a song-wide register from already charted notes.
#[derive(Clone, Copy, Debug)]
pub struct ReductionPolicy {
    pub min_gap_seconds: f64,
    pub max_chord: usize,
    pub lanes: [u8; 5],
    pub sustain_gap_divisor: u32,
    pub min_sustain_seconds: f64,
}

pub fn reduction_policy(tier: usize, bass: bool) -> ReductionPolicy {
    let tier = tier.min(3);
    ReductionPolicy {
        min_gap_seconds: [0.30, 0.16, 0.08, if bass { 0.055 } else { 0.0 }][tier],
        max_chord: [1, 1, 2, 2][tier],
        lanes: match tier {
            0 => [0, 1, 1, 2, 2],
            1 => [0, 1, 2, 2, 3],
            _ => [0, 1, 2, 3, 4],
        },
        sustain_gap_divisor: 8,
        min_sustain_seconds: 0.20,
    }
}

fn pitched(events: &[PitchNote], timing: &TempoMap, tier: usize, bass: bool) -> Vec<Note> {
    pitched_with_sources(events, timing, tier, bass).0
}

/// The raw candidate actually selected for each Expert Bass gem. Review must
/// not substitute a nearer discarded octave/harmonic detection for this source.
pub type PitchSources = BTreeMap<u64, Vec<(u8, PitchNote)>>;

pub fn bass_attack_sources(events: &[PitchNote], timing: &TempoMap) -> PitchSources {
    pitched_with_sources(events, timing, 3, true).1
}

pub fn guitar_attack_sources(events: &[PitchNote], timing: &TempoMap) -> PitchSources {
    pitched_with_sources(events, timing, 3, false).1
}

/// Only monophonic chart attacks whose lane matches the effective mapper.
/// A five-fret color alone cannot recover pitch or establish authored identity.
pub fn bass_monophonic_chart_sources(
    events: &[PitchNote],
    timing: &TempoMap,
    notes: &[Note],
) -> BTreeMap<u64, PitchNote> {
    let sources = bass_attack_sources(events, timing);
    let mut chart_lanes: BTreeMap<u64, BTreeSet<u8>> = BTreeMap::new();
    for note in notes.iter().filter(|note| note.lane <= 4) {
        chart_lanes.entry(note.tick).or_default().insert(note.lane);
    }
    chart_lanes
        .into_iter()
        .filter_map(|(tick, lanes)| {
            if lanes.len() != 1 {
                return None;
            }
            let (_, source) = sources
                .iter()
                .filter(|(source_tick, source)| {
                    source.len() == 1
                        && lanes.contains(&source[0].0)
                        && (timing.seconds(**source_tick) - timing.seconds(tick)).abs() <= 0.026
                })
                .min_by(|(left, _), (right, _)| {
                    (timing.seconds(**left) - timing.seconds(tick))
                        .abs()
                        .total_cmp(&(timing.seconds(**right) - timing.seconds(tick)).abs())
                })?;
            Some((tick, source[0].1.clone()))
        })
        .collect()
}

fn pitched_with_sources(
    events: &[PitchNote],
    timing: &TempoMap,
    tier: usize,
    bass: bool,
) -> (Vec<Note>, PitchSources) {
    let mut events: Vec<_> = events
        .iter()
        .filter(|n| {
            n.confidence >= 0.20
                && if bass {
                    (24..=67).contains(&n.pitch)
                } else {
                    n.pitch >= 40
                }
        })
        .collect();
    if events.is_empty() {
        return (Vec::new(), BTreeMap::new());
    }
    events.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    let mut grouped: BTreeMap<u64, Vec<&PitchNote>> = BTreeMap::new();
    // Keep raw attack timing. Quantization is not silently forced onto swing,
    // live timing or an existing author's tempo map.
    for n in events {
        if let Some(tick) = timing.tick(n.start) {
            let nearest = grouped.range(..=tick).next_back().map(|(t, _)| *t);
            let group = nearest
                .filter(|t| timing.seconds(tick) - timing.seconds(*t) <= 0.025)
                .unwrap_or(tick);
            grouped.entry(group).or_default().push(n);
        }
    }
    // Discarded weak co-onset bass harmonics must not widen the whole-song
    // register and compress the actual bass line into fewer frets. Calculate
    // this once from the same eligible attack population for every tier.
    let mut pitches: Vec<_> = grouped
        .values()
        .flat_map(|group| {
            let strongest = group.iter().map(|note| note.confidence).fold(0., f32::max);
            group
                .iter()
                .filter(move |note| !bass || note.confidence >= strongest * 0.65)
                .map(|note| note.pitch)
        })
        .collect();
    if pitches.is_empty() {
        return (Vec::new(), BTreeMap::new());
    }
    pitches.sort_unstable();
    let low = percentile(&pitches, 0.05);
    let high = percentile(&pitches, 0.95);
    let fret = |pitch: u8| -> u8 {
        if high <= low {
            0
        } else {
            (((pitch as f64 - low) / (high - low) * 5.0).floor() as i32).clamp(0, 4) as u8
        }
    };
    // Expert bass still needs a strummable attack sequence. This floor removes
    // 25–50 ms retrigger fragments; it does not affect imported/authored tracks.
    let policy = reduction_policy(tier, bass);
    let min_gap = policy.min_gap_seconds;
    let max_chord = policy.max_chord;
    let mut positions: Vec<(u64, Vec<(u8, u64)>)> = Vec::new();
    let mut sources: BTreeMap<u64, Vec<(u8, PitchNote)>> = BTreeMap::new();
    let mut last = None;
    for (tick, mut group) in grouped {
        if last.is_some_and(|prev| timing.seconds(tick) - timing.seconds(prev) < min_gap) {
            continue;
        }
        group.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(a.pitch.cmp(&b.pitch))
        });
        let strongest = group[0].confidence;
        let mut lanes = BTreeMap::new();
        for note in group {
            if note.confidence < strongest * 0.65 {
                continue;
            }
            let mut lane = fret(note.pitch);
            // chartgen's order-preserving lower-difficulty lane compression.
            lane = policy.lanes[lane as usize];
            let length = timing.tick(note.end).unwrap_or(tick).saturating_sub(tick);
            if !lanes.contains_key(&lane) {
                sources.entry(tick).or_default().push((lane, note.clone()));
            }
            lanes
                .entry(lane)
                .and_modify(|old: &mut u64| *old = (*old).max(length))
                .or_insert(length);
            if lanes.len() >= max_chord {
                break;
            }
        }
        positions.push((tick, lanes.into_iter().collect()));
        last = Some(tick);
    }
    let mut out = Vec::new();
    for (i, (tick, lanes)) in positions.iter().enumerate() {
        let next = positions.get(i + 1).map(|(t, _)| *t);
        for (lane, duration) in lanes {
            let duration =
                next.map_or(*duration, |n| {
                    (*duration).min(n.saturating_sub(*tick).saturating_sub(
                        (timing.resolution / policy.sustain_gap_divisor).max(1) as u64,
                    ))
                });
            let sustain = if timing.seconds(tick.saturating_add(duration)) - timing.seconds(*tick)
                >= policy.min_sustain_seconds
            {
                duration
            } else {
                0
            };
            out.push(Note {
                tick: *tick,
                lane: *lane,
                sustain,
            });
        }
    }
    out.sort();
    (out, sources)
}

#[cfg(test)]
mod bass_register_tests {
    use super::*;

    #[test]
    fn discarded_upper_harmonics_do_not_compress_the_bass_register() {
        let timing = TempoMap::constant(120.).unwrap();
        let original = vec![
            PitchNote {
                start: 0.2,
                end: 0.45,
                pitch: 40,
                confidence: 0.9,
            },
            PitchNote {
                start: 0.7,
                end: 0.95,
                pitch: 42,
                confidence: 0.9,
            },
        ];
        let mut with_harmonics = original.clone();
        with_harmonics.extend(original.iter().map(|note| PitchNote {
            pitch: note.pitch + 12,
            confidence: 0.3,
            ..note.clone()
        }));
        for tier in 0..4 {
            assert_eq!(
                pitched(&original, &timing, tier, true),
                pitched(&with_harmonics, &timing, tier, true)
            );
        }
        let expert = pitched(&original, &timing, 3, true);
        assert_eq!(
            expert.iter().map(|note| note.lane).collect::<Vec<_>>(),
            [0, 4]
        );
        let sources = bass_monophonic_chart_sources(&with_harmonics, &timing, &expert);
        assert_eq!(
            sources.values().map(|note| note.pitch).collect::<Vec<_>>(),
            [40, 42]
        );
        let wrong_lane: Vec<_> = expert
            .iter()
            .map(|note| Note {
                lane: 2,
                ..note.clone()
            })
            .collect();
        assert!(bass_monophonic_chart_sources(&with_harmonics, &timing, &wrong_lane).is_empty());
    }
}
