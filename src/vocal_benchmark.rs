//! Private, reference-bound evaluation. Generated notes never become truth.
use crate::{clean::AlignedWord, models, vocal_generation::Generation, vocals::VocalTone};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub version: u32,
    pub recordings: Vec<Recording>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub id: String,
    pub artist: String,
    pub language: String,
    pub split: String,
    pub categories: Vec<String>,
    pub audio: PathBuf,
    pub audio_sha256: String,
    pub reference: Option<PathBuf>,
    pub baseline: Option<PathBuf>,
    pub candidate: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub reference_kind: String,
    pub audio_sha256: String,
    pub duration_seconds: f64,
    /// Named published annotation or explicit human review provenance.
    pub provenance: String,
    pub reviewed: bool,
    pub frames: Vec<ReferenceFrame>,
    pub notes: Vec<ReferenceNote>,
    pub words: Vec<AlignedWord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceFrame {
    pub time_seconds: f64,
    pub midi: Option<f64>,
    #[serde(default)]
    pub backing_midi: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceNote {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub midi: u8,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Metrics {
    pub reference_voiced_frames: usize,
    pub missed_voiced_frames: usize,
    pub reference_unvoiced_frames: usize,
    pub correct_pitch_frames: usize,
    pub incorrect_pitch_frames: usize,
    pub detected_voiced_frames: usize,
    pub false_voiced_frames: usize,
    pub octave_error_frames: usize,
    pub backing_capture_frames: usize,
    pub lead_backing_switches: usize,
    pub note_matches: usize,
    pub reference_notes: usize,
    pub predicted_notes: usize,
    pub matched_onset_error_seconds: f64,
    pub matched_offset_error_seconds: f64,
    pub word_errors: usize,
    pub reference_words: usize,
    pub word_boundary_matches: usize,
    pub word_boundary_absolute_error_seconds: f64,
}
impl Metrics {
    fn pitch_accuracy(&self) -> f64 {
        self.correct_pitch_frames as f64 / self.reference_voiced_frames.max(1) as f64
    }
    fn voiced_recall(&self) -> f64 {
        self.detected_voiced_frames as f64 / self.reference_voiced_frames.max(1) as f64
    }
    fn error_rate(&self) -> f64 {
        (self.incorrect_pitch_frames + self.missed_voiced_frames) as f64
            / self.reference_voiced_frames.max(1) as f64
    }
    fn wrong_pitch_proportion(&self) -> f64 {
        self.incorrect_pitch_frames as f64 / self.detected_voiced_frames.max(1) as f64
    }
    fn false_voicing_rate(&self) -> f64 {
        self.false_voiced_frames as f64 / self.reference_unvoiced_frames.max(1) as f64
    }
}
#[derive(Serialize)]
pub struct Comparison {
    pub id: String,
    pub language: String,
    pub split: String,
    pub baseline: Metrics,
    pub candidate: Metrics,
    pub baseline_notes: Metrics,
    pub candidate_notes: Metrics,
}
#[derive(Serialize)]
pub struct Evaluation {
    pub version: u32,
    pub status: String,
    pub pending: Vec<String>,
    pub comparisons: Vec<Comparison>,
    pub promotion_eligible: bool,
    pub gate_failures: Vec<String>,
    pub limitations: Vec<String>,
}

pub fn validate_corpus(corpus: &Corpus) -> Result<()> {
    ensure!(
        corpus.version == 1 && corpus.recordings.len() == 12,
        "Benchmark requires exactly 12 recordings"
    );
    let mut ids = BTreeSet::new();
    let mut artists = BTreeSet::new();
    for recording in &corpus.recordings {
        ensure!(
            !recording.id.trim().is_empty() && ids.insert(&recording.id),
            "Duplicate/empty recording ID"
        );
        ensure!(
            !recording.artist.trim().is_empty()
                && artists.insert(recording.artist.trim().to_lowercase()),
            "Each recording needs a distinct artist, including across development and held-out splits"
        );
        ensure!(
            valid_language(&recording.language)
                && matches!(recording.split.as_str(), "development" | "held_out"),
            "Unsupported language/split"
        );
        ensure!(
            recording.audio_sha256.len() == 64
                && recording
                    .audio_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "Invalid source hash"
        );
    }
    ensure!(
        corpus
            .recordings
            .iter()
            .filter(|r| r.split == "development")
            .count()
            == 4
            && corpus
                .recordings
                .iter()
                .filter(|r| r.split == "held_out")
                .count()
                == 8,
        "Require four development and eight held-out recordings; language balance is optional"
    );
    Ok(())
}

fn valid_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 35
        && language
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'-')
}

fn validate_reference(reference: &Reference) -> Result<()> {
    ensure!(
        matches!(
            reference.reference_kind.as_str(),
            "authored_chart" | "performed_contour"
        ),
        "Unknown reference kind"
    );
    ensure!(
        reference.reviewed && !reference.provenance.trim().is_empty(),
        "Reference needs published/manual review provenance"
    );
    ensure!(
        reference.duration_seconds.is_finite()
            && (0.0..=3600.).contains(&reference.duration_seconds)
            && reference.duration_seconds > 0.,
        "Invalid reference clock"
    );
    ensure!(
        !reference.frames.is_empty()
            && reference.frames.len() <= 360_001
            && !reference.notes.is_empty()
            && reference.notes.len() <= 200_000
            && reference.words.len() <= 20_000,
        "Reference needs bounded pitch frames and notes"
    );
    ensure!(
        reference.frames.iter().all(|f| f.time_seconds.is_finite()
            && f.time_seconds >= 0.
            && f.time_seconds < reference.duration_seconds
            && f.midi
                .is_none_or(|p| p.is_finite() && (0.0..=127.).contains(&p))
            && f.backing_midi
                .iter()
                .all(|p| p.is_finite() && (0.0..=127.).contains(p)))
            && reference
                .frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds),
        "Invalid/unsorted reference frames"
    );
    ensure!(
        reference.frames.iter().any(|f| f.midi.is_some()),
        "Reference must include voiced pitch"
    );
    ensure!(
        reference.notes.iter().all(|n| n.start_seconds.is_finite()
            && n.end_seconds.is_finite()
            && n.start_seconds >= 0.
            && n.start_seconds < n.end_seconds
            && n.end_seconds <= reference.duration_seconds)
            && reference.notes.windows(2).all(|w| {
                if reference.reference_kind == "performed_contour" {
                    // Published human segmentations can overlap at transitions.
                    // Their frame-level F0 remains independent ground truth.
                    w[0].start_seconds <= w[1].start_seconds
                } else {
                    w[0].end_seconds <= w[1].start_seconds
                }
            }),
        "Invalid or unsorted reference notes, or overlapping authored chart"
    );
    Ok(())
}

pub fn score(reference: &Reference, report: &Generation) -> Result<Metrics> {
    validate_reference(reference)?;
    ensure!(
        reference
            .audio_sha256
            .eq_ignore_ascii_case(&report.audio_sha256)
            && (reference.duration_seconds - report.duration_seconds).abs() <= 1. / 16000.,
        "Reference/report recording clock differs"
    );
    ensure!(
        !report.pitch_frames.is_empty()
            && report.pitch_frames.len() <= 400_000
            && report
                .pitch_frames
                .iter()
                .all(|f| f.time_seconds.is_finite()
                    && f.time_seconds >= 0.
                    && f.time_seconds <= report.duration_seconds
                    && f.midi
                        .is_none_or(|p| p.is_finite() && (0.0..=127.).contains(&p)))
            && report
                .pitch_frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds),
        "Invalid prediction frames"
    );
    let mut metrics = Metrics::default();
    let mut role = None;
    for expected in &reference.frames {
        let i = report
            .pitch_frames
            .partition_point(|f| f.time_seconds < expected.time_seconds);
        let closest = [i.saturating_sub(1), i.min(report.pitch_frames.len() - 1)]
            .into_iter()
            .min_by(|&a, &b| {
                (report.pitch_frames[a].time_seconds - expected.time_seconds)
                    .abs()
                    .total_cmp(&(report.pitch_frames[b].time_seconds - expected.time_seconds).abs())
            })
            .unwrap();
        let predicted = &report.pitch_frames[closest];
        let pitch = if (predicted.time_seconds - expected.time_seconds).abs() <= 0.020 {
            predicted.midi
        } else {
            None
        };
        if let Some(truth) = expected.midi {
            metrics.reference_voiced_frames += 1;
            if let Some(pitch) = pitch {
                metrics.detected_voiced_frames += 1;
                let error = (pitch - truth).abs();
                let captured = error > 0.5
                    && expected
                        .backing_midi
                        .iter()
                        .any(|p| (pitch - p).abs() <= 0.5);
                let new_role = if error <= 0.5 {
                    Some(false)
                } else if captured {
                    Some(true)
                } else {
                    None
                };
                if role.zip(new_role).is_some_and(|(a, b)| a != b) {
                    metrics.lead_backing_switches += 1;
                }
                role = new_role;
                if error <= 0.5 {
                    metrics.correct_pitch_frames += 1;
                } else {
                    metrics.incorrect_pitch_frames += 1;
                }
                if (error - 12.).abs() <= 0.5 {
                    metrics.octave_error_frames += 1;
                }
                if captured {
                    metrics.backing_capture_frames += 1;
                }
            } else {
                metrics.missed_voiced_frames += 1;
                role = None;
            }
        } else {
            metrics.reference_unvoiced_frames += 1;
            metrics.false_voiced_frames += usize::from(pitch.is_some());
            role = None;
        }
    }
    if let Some(chart) = &report.chart {
        chart.validate()?;
        let timing = report
            .timing
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Scoring notes requires the saved source tempo map"))?;
        let notes: Vec<_> = chart
            .notes
            .iter()
            .filter_map(|n| match n.tone {
                VocalTone::Pitched { midi } => Some(ReferenceNote {
                    start_seconds: timing.seconds(n.start),
                    end_seconds: timing.seconds(n.end),
                    midi,
                }),
                _ => None,
            })
            .collect();
        metrics.predicted_notes = notes.len();
        let mut matched = vec![false; notes.len()];
        for expected in &reference.notes {
            let best = notes
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    !matched[*i]
                        && n.midi == expected.midi
                        && (n.start_seconds - expected.start_seconds).abs() <= 0.05
                        && (n.end_seconds - expected.end_seconds).abs()
                            <= 0.05f64.max(0.2 * (expected.end_seconds - expected.start_seconds))
                })
                .min_by(|a, b| {
                    (a.1.start_seconds - expected.start_seconds)
                        .abs()
                        .total_cmp(&(b.1.start_seconds - expected.start_seconds).abs())
                });
            if let Some((index, note)) = best {
                matched[index] = true;
                metrics.note_matches += 1;
                metrics.matched_onset_error_seconds +=
                    (note.start_seconds - expected.start_seconds).abs();
                metrics.matched_offset_error_seconds +=
                    (note.end_seconds - expected.end_seconds).abs();
            }
        }
    }
    metrics.reference_notes = reference.notes.len();
    let heard = report
        .words
        .as_ref()
        .map(|w| w.words.as_slice())
        .unwrap_or(&[]);
    let truth: Vec<_> = reference
        .words
        .iter()
        .filter_map(|w| {
            w.text
                .as_ref()
                .map(|t| (crate::clean::tokens(t).join(" "), w))
        })
        .collect();
    let predicted: Vec<_> = heard
        .iter()
        .filter_map(|w| {
            w.text
                .as_ref()
                .map(|t| (crate::clean::tokens(t).join(" "), w))
        })
        .collect();
    ensure!(
        truth.len().saturating_mul(predicted.len()) <= 4_000_000
            && truth.len() <= 5000
            && predicted.len() <= 5000,
        "Word comparison exceeds bounded alignment budget; evaluate shorter excerpts"
    );
    let mut distance = vec![vec![0usize; predicted.len() + 1]; truth.len() + 1];
    for (i, row) in distance.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, v) in distance[0].iter_mut().enumerate() {
        *v = j;
    }
    for i in 1..=truth.len() {
        for j in 1..=predicted.len() {
            distance[i][j] = (distance[i - 1][j] + 1)
                .min(distance[i][j - 1] + 1)
                .min(distance[i - 1][j - 1] + usize::from(truth[i - 1].0 != predicted[j - 1].0));
        }
    }
    metrics.word_errors = distance[truth.len()][predicted.len()];
    metrics.reference_words = truth.len();
    let (mut i, mut j) = (truth.len(), predicted.len());
    while i > 0 && j > 0 {
        let same = truth[i - 1].0 == predicted[j - 1].0;
        if distance[i][j] == distance[i - 1][j - 1] + usize::from(!same) {
            if same
                && let (Some((a, b)), Some((c, d))) = (
                    truth[i - 1].1.start.zip(truth[i - 1].1.end),
                    predicted[j - 1].1.start.zip(predicted[j - 1].1.end),
                )
            {
                ensure!(
                    [a, b, c, d].iter().all(|v| v.is_finite()),
                    "Nonfinite word boundaries"
                );
                metrics.word_boundary_matches += 1;
                metrics.word_boundary_absolute_error_seconds += (a - c).abs() + (b - d).abs();
            }
            i -= 1;
            j -= 1;
        } else if distance[i][j] == distance[i - 1][j] + 1 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    Ok(metrics)
}

/// Sample the actual editable/exported notes on the reference clock. Detector
/// frames remain a separate metric; uncertainty never excludes a chart note.
pub fn score_exported_notes(reference: &Reference, report: &Generation) -> Result<Metrics> {
    validate_reference(reference)?;
    let mut selected = report.clone();
    let template = report
        .pitch_frames
        .first()
        .context("Missing analysis frame clock")?;
    let mut intervals = vec![];
    if let Some(chart) = &report.chart {
        chart.validate()?;
        let timing = report
            .timing
            .as_ref()
            .context("Exported-note scoring needs source timing")?;
        let canonical =
            crate::timing::TempoMap::new(timing.resolution, timing.offset, timing.tempos.clone())?;
        ensure!(
            canonical == *timing,
            "Exported-note timing is not a canonical source map"
        );
        for n in &chart.notes {
            if let VocalTone::Pitched { midi } = n.tone {
                intervals.push((timing.seconds(n.start), timing.seconds(n.end), midi))
            }
        }
    }
    selected.pitch_frames = reference
        .frames
        .iter()
        .map(|r| {
            let mut f = template.clone();
            f.time_seconds = r.time_seconds;
            let at = intervals.partition_point(|(start, _, _)| *start <= r.time_seconds);
            f.midi = at.checked_sub(1).and_then(|i| {
                let (_, end, pitch) = intervals[i];
                (r.time_seconds < end).then_some(pitch as f64)
            });
            f
        })
        .collect();
    score(reference, &selected)
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    ensure!(
        fs::metadata(path)?.len() <= 128 * 1024 * 1024,
        "Benchmark JSON exceeds 128 MiB"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn evaluate(path: &Path) -> Result<Evaluation> {
    evaluate_inner(path, false)
}
/// Explicit diagnostic subset; can never pass the promotion gate.
pub fn evaluate_partial(path: &Path) -> Result<Evaluation> {
    evaluate_inner(path, true)
}
fn evaluate_inner(path: &Path, partial: bool) -> Result<Evaluation> {
    let corpus: Corpus = read(path)?;
    if partial {
        ensure!(
            corpus.version == 1 && !corpus.recordings.is_empty() && corpus.recordings.len() <= 12,
            "Invalid diagnostic corpus size"
        );
        let mut ids = BTreeSet::new();
        let mut artists = BTreeSet::new();
        for recording in &corpus.recordings {
            ensure!(
                ids.insert(&recording.id)
                    && artists.insert(recording.artist.trim().to_lowercase())
                    && valid_language(&recording.language)
                    && matches!(recording.split.as_str(), "development" | "held_out"),
                "Invalid diagnostic identities/language/splits"
            );
        }
    } else {
        validate_corpus(&corpus)?;
    }
    let base = path.parent().unwrap_or(Path::new("."));
    let mut evaluation=Evaluation{version:1,status:"needs_annotations_or_runs".into(),pending:vec![],comparisons:vec![],promotion_eligible:false,gate_failures:vec![],limitations:vec!["Automated results do not establish listening, browser/native GUI or game acceptance.".into(),"Runtime and peak memory require separate native run receipts; absent measurements are not estimated.".into()]};
    for recording in &corpus.recordings {
        ensure!(
            models::hash_file(&base.join(&recording.audio))?
                .eq_ignore_ascii_case(&recording.audio_sha256),
            "Benchmark audio changed: {}",
            recording.id
        );
        let (Some(reference), Some(baseline), Some(candidate)) = (
            &recording.reference,
            &recording.baseline,
            &recording.candidate,
        ) else {
            evaluation.pending.push(recording.id.clone());
            continue;
        };
        let reference: Reference = read(&base.join(reference))?;
        ensure!(
            reference
                .audio_sha256
                .eq_ignore_ascii_case(&recording.audio_sha256),
            "Annotation belongs to another recording"
        );
        let baseline: Generation = read(&base.join(baseline))?;
        let candidate: Generation = read(&base.join(candidate))?;
        evaluation.comparisons.push(Comparison {
            id: recording.id.clone(),
            language: recording.language.clone(),
            split: recording.split.clone(),
            baseline: score(&reference, &baseline)?,
            candidate: score(&reference, &candidate)?,
            baseline_notes: score_exported_notes(&reference, &baseline)?,
            candidate_notes: score_exported_notes(&reference, &candidate)?,
        });
    }
    if partial {
        evaluation.status = "diagnostic_subset_no_promotion".into();
        evaluation.gate_failures.push("Explicit partial diagnostic: twelve recordings and full pitch-category coverage are required for promotion".into());
        return Ok(evaluation);
    }
    if !evaluation.pending.is_empty() {
        return Ok(evaluation);
    }
    for category in ["solo", "harmony", "rap"] {
        if !corpus
            .recordings
            .iter()
            .any(|r| r.categories.iter().any(|c| c == category))
        {
            evaluation
                .gate_failures
                .push(format!("Missing annotated coverage: {category}"));
        }
    }
    let held: Vec<_> = evaluation
        .comparisons
        .iter()
        .filter(|r| r.split == "held_out")
        .collect();
    let mean =
        |candidate: bool, language: Option<&str>, notes: bool, metric: fn(&Metrics) -> f64| {
            let rows: Vec<_> = held
                .iter()
                .filter(|r| language.is_none_or(|l| r.language == l))
                .collect();
            rows.iter()
                .map(|r| {
                    metric(match (candidate, notes) {
                        (true, true) => &r.candidate_notes,
                        (false, true) => &r.baseline_notes,
                        (true, false) => &r.candidate,
                        (false, false) => &r.baseline,
                    })
                })
                .sum::<f64>()
                / rows.len().max(1) as f64
        };
    for (notes, label) in [(false, "detected contour"), (true, "exported notes")] {
        let before = mean(false, None, notes, Metrics::error_rate);
        let after = mean(true, None, notes, Metrics::error_rate);
        if before <= 0. || after > before * 0.8 {
            evaluation.gate_failures.push(
            format!("Need at least 20% relative reduction in missed-or-wrong pitched frames on held-out recordings ({label})"),
        );
        }
        if mean(true, None, notes, Metrics::wrong_pitch_proportion)
            >= mean(false, None, notes, Metrics::wrong_pitch_proportion)
        {
            evaluation.gate_failures.push(format!(
                "The proportion of detected pitches that are wrong must improve ({label})"
            ));
        }
        if mean(true, None, notes, Metrics::false_voicing_rate)
            > mean(false, None, notes, Metrics::false_voicing_rate) + 0.02
        {
            evaluation.gate_failures.push(format!(
                "False voicing increased by more than two percentage points ({label})"
            ));
        }
        if mean(true, None, notes, Metrics::voiced_recall) + 0.02
            < mean(false, None, notes, Metrics::voiced_recall)
        {
            evaluation.gate_failures.push(format!(
                "Voiced recall fell by more than two percentage points ({label})"
            ));
        }
        let languages: BTreeSet<_> = held.iter().map(|r| r.language.as_str()).collect();
        for language in languages {
            if mean(true, Some(language), notes, Metrics::pitch_accuracy) + 0.02
                < mean(false, Some(language), notes, Metrics::pitch_accuracy)
            {
                evaluation.gate_failures.push(format!(
                    "Pitch accuracy regressed by more than two points in {language} ({label})"
                ));
            }
        }
    }
    evaluation.promotion_eligible = evaluation.gate_failures.is_empty();
    evaluation.status = if evaluation.promotion_eligible {
        "automated_gate_passed_acceptance_pending"
    } else {
        "experimental_gate_failed"
    }
    .into();
    Ok(evaluation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn performed_annotations_keep_overlapping_boundaries_and_original_f0() {
        let mut r = Reference {
            reference_kind: "performed_contour".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 1.,
            provenance: "Synthetic publisher-style note boundaries and independent F0".into(),
            reviewed: true,
            frames: vec![
                ReferenceFrame {
                    time_seconds: 0.3,
                    midi: Some(60.),
                    backing_midi: vec![],
                },
                ReferenceFrame {
                    time_seconds: 0.6,
                    midi: Some(62.),
                    backing_midi: vec![],
                },
            ],
            notes: vec![
                ReferenceNote {
                    start_seconds: 0.1,
                    end_seconds: 0.55,
                    midi: 60,
                },
                ReferenceNote {
                    start_seconds: 0.5,
                    end_seconds: 0.9,
                    midi: 62,
                },
            ],
            words: vec![],
        };
        let before = serde_json::to_vec(&r).unwrap();
        validate_reference(&r).unwrap();
        assert_eq!(before, serde_json::to_vec(&r).unwrap());
        r.reference_kind = "authored_chart".into();
        assert!(validate_reference(&r).is_err());
        r.reference_kind = "performed_contour".into();
        r.notes.swap(0, 1);
        assert!(validate_reference(&r).is_err());
    }
    #[test]
    fn missing_notes_cannot_make_pitch_error_disappear() {
        let mut m = Metrics {
            reference_voiced_frames: 100,
            correct_pitch_frames: 80,
            incorrect_pitch_frames: 20,
            detected_voiced_frames: 100,
            ..Default::default()
        };
        assert_eq!(m.error_rate(), 0.2);
        m.incorrect_pitch_frames = 0;
        m.missed_voiced_frames = 20;
        m.detected_voiced_frames = 80;
        assert_eq!(m.error_rate(), 0.2);
        m.missed_voiced_frames = 100;
        m.correct_pitch_frames = 0;
        m.detected_voiced_frames = 0;
        assert_eq!(m.error_rate(), 1.);
    }
    #[test]
    fn corpus_requires_distinct_artists_and_frozen_splits_not_language_balance() {
        let mut corpus = Corpus {
            version: 1,
            recordings: (0..12)
                .map(|i| Recording {
                    id: format!("song{i}"),
                    artist: format!("artist{i}"),
                    language: if i < 6 { "en" } else { "es" }.into(),
                    split: if i % 6 < 2 { "development" } else { "held_out" }.into(),
                    categories: vec![],
                    audio: "audio.wav".into(),
                    audio_sha256: "a".repeat(64),
                    reference: None,
                    baseline: None,
                    candidate: None,
                })
                .collect(),
        };
        validate_corpus(&corpus).unwrap();
        for recording in &mut corpus.recordings {
            recording.language = "en".into();
        }
        validate_corpus(&corpus).unwrap();
        corpus.recordings[11].artist = corpus.recordings[0].artist.clone();
        assert!(validate_corpus(&corpus).is_err());
    }
}
