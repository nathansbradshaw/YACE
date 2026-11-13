//! Annotated musical notes and performed F0 are different reference objects.
//! Grade note pitch/presence by interval duration, without sampling-boundary
//! aliases or replacing the independently reported continuous-F0 diagnostics.
use super::{Corpus, Reference, ReferenceNote, read, validate_reference};
use crate::{timing::TempoMap, vocal_generation::Generation, vocals::VocalTone};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct NoteIntervalMetrics {
    pub reference_basis: &'static str,
    pub reference_pitched_seconds: f64,
    pub reference_gap_seconds: f64,
    pub correct_pitch_seconds: f64,
    pub wrong_pitch_seconds: f64,
    pub missed_pitched_seconds: f64,
    pub detected_in_reference_seconds: f64,
    pub false_pitched_seconds: f64,
    pub octave_error_seconds: f64,
    /// Report separately instead of silently ignoring notes outside the audio.
    pub outside_recording_pitched_seconds: f64,
    /// Only arithmetic overlap up to one nanosecond is canonicalized.
    pub normalized_boundary_overlaps: usize,
    pub pitch_accuracy: f64,
    pub note_presence_recall: f64,
    pub missing_or_wrong_rate: f64,
    pub wrong_pitch_proportion: f64,
    pub false_note_time_rate: f64,
}

/// Read-only comparison against reviewed note intervals, never generated truth.
/// Continuous F0 is neither quantized nor replaced. Reference-note overlaps
/// larger than floating-point boundary noise require adjudication and fail.
/// Lyrics do not participate. This diagnostic does not promote a backend.
pub fn score_note_intervals(
    reference: &Reference,
    report: &Generation,
) -> Result<NoteIntervalMetrics> {
    validate_reference(reference)?;
    ensure!(
        report.version == 1
            && reference
                .audio_sha256
                .eq_ignore_ascii_case(&report.audio_sha256)
            && (reference.duration_seconds - report.duration_seconds).abs() <= 1. / 16000.,
        "Note-interval reference/report source or clock differs"
    );
    let mut expected = reference.notes.clone();
    ensure!(
        expected.iter().all(|n| n.midi <= 127),
        "Invalid reference MIDI pitch"
    );
    let mut normalized = 0;
    for i in 0..expected.len().saturating_sub(1) {
        let next = expected[i + 1].start_seconds;
        if expected[i].end_seconds > next {
            ensure!(
                // Compare endpoints so subtraction roundoff at a published
                // one-nanosecond boundary cannot turn it into a real overlap.
                expected[i].end_seconds <= next + 1e-9,
                "Overlapping annotated note targets need adjudication; no pitch was chosen from the overlap"
            );
            expected[i].end_seconds = next;
            normalized += 1;
        }
    }
    ensure!(
        expected.iter().all(|n| n.start_seconds < n.end_seconds),
        "Invalid canonical note targets"
    );
    let duration = reference.duration_seconds;
    let mut predicted = vec![];
    let mut outside = 0.;
    if let Some(chart) = &report.chart {
        chart.validate()?;
        let timing = report
            .timing
            .as_ref()
            .context("Note intervals require the original MIDI clock")?;
        ensure!(
            chart.notes.len() <= 200_000 && chart.resolution == timing.resolution,
            "Unsupported note count or mismatched MIDI resolution"
        );
        ensure!(
            TempoMap::new(timing.resolution, timing.offset, timing.tempos.clone())? == *timing,
            "Noncanonical note-interval MIDI clock"
        );
        for note in &chart.notes {
            if let VocalTone::Pitched { midi } = note.tone {
                let start = timing.seconds(note.start);
                let end = timing.seconds(note.end);
                ensure!(
                    start.is_finite() && end.is_finite() && start < end,
                    "Invalid note-interval clock"
                );
                let a = start.max(0.);
                let b = end.min(duration);
                let inside = (b - a).max(0.);
                outside += (end - start - inside).max(0.);
                if a < b {
                    predicted.push(ReferenceNote {
                        start_seconds: a,
                        end_seconds: b,
                        midi,
                    });
                }
            }
        }
    }
    let mut boundaries = vec![0., duration];
    for n in expected.iter().chain(&predicted) {
        boundaries.extend([n.start_seconds, n.end_seconds]);
    }
    boundaries.sort_by(f64::total_cmp);
    boundaries.dedup();
    let mut m = NoteIntervalMetrics {
        reference_basis: "reviewed_note_intervals_v1",
        reference_pitched_seconds: 0.,
        reference_gap_seconds: 0.,
        correct_pitch_seconds: 0.,
        wrong_pitch_seconds: 0.,
        missed_pitched_seconds: 0.,
        detected_in_reference_seconds: 0.,
        false_pitched_seconds: 0.,
        octave_error_seconds: 0.,
        outside_recording_pitched_seconds: outside,
        normalized_boundary_overlaps: normalized,
        pitch_accuracy: 0.,
        note_presence_recall: 0.,
        missing_or_wrong_rate: 0.,
        wrong_pitch_proportion: 0.,
        false_note_time_rate: 0.,
    };
    let (mut i, mut j) = (0, 0);
    for w in boundaries.windows(2) {
        let at = w[0] + (w[1] - w[0]) / 2.;
        let length = w[1] - w[0];
        while i < expected.len() && expected[i].end_seconds <= at {
            i += 1;
        }
        while j < predicted.len() && predicted[j].end_seconds <= at {
            j += 1;
        }
        let truth = expected
            .get(i)
            .filter(|n| n.start_seconds <= at && at < n.end_seconds)
            .map(|n| n.midi);
        let pitch = predicted
            .get(j)
            .filter(|n| n.start_seconds <= at && at < n.end_seconds)
            .map(|n| n.midi);
        match (truth, pitch) {
            (Some(a), Some(b)) => {
                m.reference_pitched_seconds += length;
                m.detected_in_reference_seconds += length;
                if a == b {
                    m.correct_pitch_seconds += length;
                } else {
                    m.wrong_pitch_seconds += length;
                    if a.abs_diff(b) == 12 {
                        m.octave_error_seconds += length;
                    }
                }
            }
            (Some(_), None) => {
                m.reference_pitched_seconds += length;
                m.missed_pitched_seconds += length;
            }
            (None, Some(_)) => {
                m.reference_gap_seconds += length;
                m.false_pitched_seconds += length;
            }
            (None, None) => m.reference_gap_seconds += length,
        }
    }
    ensure!(
        m.reference_pitched_seconds > 0.,
        "No pitched note-target duration"
    );
    m.pitch_accuracy = (m.correct_pitch_seconds / m.reference_pitched_seconds).clamp(0., 1.);
    m.note_presence_recall =
        (m.detected_in_reference_seconds / m.reference_pitched_seconds).clamp(0., 1.);
    m.missing_or_wrong_rate = ((m.wrong_pitch_seconds + m.missed_pitched_seconds)
        / m.reference_pitched_seconds)
        .clamp(0., 1.);
    m.wrong_pitch_proportion = if m.detected_in_reference_seconds > 0. {
        m.wrong_pitch_seconds / m.detected_in_reference_seconds
    } else {
        0.
    };
    m.false_note_time_rate = if m.reference_gap_seconds > 0. {
        m.false_pitched_seconds / m.reference_gap_seconds
    } else {
        0.
    };
    Ok(m)
}

#[derive(Serialize)]
pub struct NoteIntervalComparison {
    pub id: String,
    pub language: String,
    pub split: String,
    pub baseline: Option<NoteIntervalMetrics>,
    pub candidate: Option<NoteIntervalMetrics>,
    /// Missing/ambiguous references are not zero errors or a silent fallback.
    pub unavailable_reason: Option<String>,
}

/// Explicit supplementary diagnostic; original F0 metrics/gates stay intact.
pub fn diagnose_note_intervals(path: &Path) -> Result<Vec<NoteIntervalComparison>> {
    let corpus: Corpus = read(path)?;
    ensure!(
        corpus.version == 1 && !corpus.recordings.is_empty() && corpus.recordings.len() <= 12,
        "Invalid note-interval diagnostic corpus"
    );
    let mut results = vec![];
    for recording in &corpus.recordings {
        let score = || -> Result<_> {
            let reference: Reference = read(
                recording
                    .reference
                    .as_ref()
                    .context("Missing note reference")?,
            )?;
            let baseline: Generation =
                read(recording.baseline.as_ref().context("Missing baseline")?)?;
            let candidate: Generation =
                read(recording.candidate.as_ref().context("Missing candidate")?)?;
            ensure!(
                reference
                    .audio_sha256
                    .eq_ignore_ascii_case(&recording.audio_sha256)
                    && crate::models::hash_file(&recording.audio)?
                        .eq_ignore_ascii_case(&recording.audio_sha256),
                "Note-interval corpus audio binding differs"
            );
            Ok((
                score_note_intervals(&reference, &baseline)?,
                score_note_intervals(&reference, &candidate)?,
            ))
        };
        let (baseline, candidate, unavailable_reason) = match score() {
            Ok((a, b)) => (Some(a), Some(b), None),
            Err(e) => (None, None, Some(format!("{e:#}"))),
        };
        results.push(NoteIntervalComparison {
            id: recording.id.clone(),
            language: recording.language.clone(),
            split: recording.split.clone(),
            baseline,
            candidate,
            unavailable_reason,
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        vocal_benchmark::{self as vb, ReferenceFrame},
        vocal_generation::{self as vg, PitchFrame, PitchSupport},
        vocals::{self, Syllable, VocalChart, VocalNote, VocalPhrase},
    };
    fn fixture() -> (Reference, Generation) {
        let timing = TempoMap::constant(120.).unwrap();
        let reference = Reference {
            reference_kind: "performed_contour".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 1.,
            provenance: "Synthetic human note and independent expressive F0".into(),
            reviewed: true,
            frames: [(0.2, 68.2), (0.4, 69.8), (0.6, 69.), (0.8, 69.7)]
                .into_iter()
                .map(|(time_seconds, midi)| ReferenceFrame {
                    time_seconds,
                    midi: Some(midi),
                    backing_midi: vec![],
                })
                .collect(),
            notes: vec![ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 0.9,
                midi: 69,
            }],
            words: vec![],
        };
        let report = Generation {
            version: 1,
            audio_sha256: reference.audio_sha256.clone(),
            vocal_stem_sha256: "b".repeat(64),
            duration_seconds: 1.,
            algorithm: "Synthetic reference MIDI".into(),
            timing: Some(timing),
            policy: vg::Policy::default(),
            status: vg::Status::ReviewRequired,
            chart: None,
            pitch_candidates: vec![],
            pitch_frames: (0..100)
                .map(|i| PitchFrame {
                    time_seconds: i as f64 * 0.01,
                    midi: Some(69.),
                    yin_midi: None,
                    mcleod_midi: None,
                    clarity: 1.,
                    rms: 0.1,
                    local_rms: None,
                    local_energy_active: None,
                    support: PitchSupport::Unspecified,
                })
                .collect(),
            words: None,
            lyrics: None,
            issues: vec![],
            warnings: vec![],
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: None,
            note_evidence: vec![],
        };
        let mut report = report;
        set_notes(&mut report, &reference.notes);
        (reference, report)
    }
    fn set_notes(g: &mut Generation, notes: &[ReferenceNote]) {
        let timing = g.timing.as_ref().unwrap();
        let notes = notes
            .iter()
            .map(|n| VocalNote {
                start: timing.tick(n.start_seconds).unwrap(),
                end: timing.tick(n.end_seconds).unwrap(),
                tone: VocalTone::Pitched { midi: n.midi },
                lyric: Syllable::Missing,
                midi: None,
            })
            .collect::<Vec<_>>();
        g.chart = notes.first().zip(notes.last()).map(|(a, b)| VocalChart {
            version: 1,
            resolution: timing.resolution,
            source: "Synthetic reviewed notes".into(),
            phrases: vec![VocalPhrase {
                start: a.start,
                end: b.end,
                player: 1,
            }],
            notes: notes.clone(),
        });
    }
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} differs from {b}");
    }
    #[test]
    fn exact_reference_midi_has_full_credit_without_rewriting_performed_f0() {
        let (r, g) = fixture();
        let before = serde_json::to_vec(&r).unwrap();
        let legacy = vb::score_exported_notes(&r, &g).unwrap();
        assert_eq!(legacy.correct_pitch_frames, 1);
        assert_eq!(legacy.incorrect_pitch_frames, 3);
        let m = score_note_intervals(&r, &g).unwrap();
        close(m.pitch_accuracy, 1.);
        close(m.note_presence_recall, 1.);
        close(m.wrong_pitch_seconds, 0.);
        close(m.false_pitched_seconds, 0.);
        close(m.reference_pitched_seconds, 0.8);
        assert_eq!(before, serde_json::to_vec(&r).unwrap());
        assert_eq!(
            vb::score_exported_notes(&r, &g)
                .unwrap()
                .incorrect_pitch_frames,
            3
        );
    }
    #[test]
    fn missing_and_wrong_notes_both_count_and_real_gaps_remain_negative_targets() {
        let (r, mut g) = fixture();
        set_notes(
            &mut g,
            &[ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 0.5,
                midi: 69,
            }],
        );
        let missed = score_note_intervals(&r, &g).unwrap();
        close(missed.missing_or_wrong_rate, 0.5);
        close(missed.note_presence_recall, 0.5);
        set_notes(
            &mut g,
            &[
                ReferenceNote {
                    start_seconds: 0.1,
                    end_seconds: 0.5,
                    midi: 69,
                },
                ReferenceNote {
                    start_seconds: 0.5,
                    end_seconds: 0.9,
                    midi: 70,
                },
            ],
        );
        let wrong = score_note_intervals(&r, &g).unwrap();
        close(wrong.missing_or_wrong_rate, missed.missing_or_wrong_rate);
        close(wrong.note_presence_recall, 1.);
        set_notes(
            &mut g,
            &[ReferenceNote {
                start_seconds: 0.,
                end_seconds: 1.,
                midi: 69,
            }],
        );
        let extra = score_note_intervals(&r, &g).unwrap();
        close(extra.false_pitched_seconds, 0.2);
        close(extra.false_note_time_rate, 1.);
        g.chart = None;
        let empty = score_note_intervals(&r, &g).unwrap();
        close(empty.missing_or_wrong_rate, 1.);
        close(empty.note_presence_recall, 0.);
    }
    #[test]
    fn internal_breath_gaps_and_octave_errors_are_counted() {
        let (mut r, mut g) = fixture();
        r.notes = vec![
            ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 0.4,
                midi: 69,
            },
            ReferenceNote {
                start_seconds: 0.6,
                end_seconds: 0.9,
                midi: 71,
            },
        ];
        set_notes(
            &mut g,
            &[
                ReferenceNote {
                    start_seconds: 0.1,
                    end_seconds: 0.4,
                    midi: 81,
                },
                ReferenceNote {
                    start_seconds: 0.4,
                    end_seconds: 0.6,
                    midi: 69,
                },
                ReferenceNote {
                    start_seconds: 0.6,
                    end_seconds: 0.9,
                    midi: 71,
                },
            ],
        );
        let m = score_note_intervals(&r, &g).unwrap();
        close(m.correct_pitch_seconds, 0.3);
        close(m.wrong_pitch_seconds, 0.3);
        close(m.octave_error_seconds, 0.3);
        close(m.false_pitched_seconds, 0.2);
        close(m.false_note_time_rate, 0.5);
    }

    #[test]
    fn boundary_float_noise_is_explicit_but_genuine_overlap_is_not_arbitrarily_scored() {
        let (mut r, mut g) = fixture();
        r.notes = vec![
            ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 0.1 + 0.2,
                midi: 69,
            },
            ReferenceNote {
                start_seconds: 0.3,
                end_seconds: 0.9,
                midi: 70,
            },
        ];
        set_notes(&mut g, &r.notes);
        let before = serde_json::to_vec(&r).unwrap();
        let m = score_note_intervals(&r, &g).unwrap();
        assert_eq!(m.normalized_boundary_overlaps, 1);
        close(m.pitch_accuracy, 1.);
        assert_eq!(before, serde_json::to_vec(&r).unwrap());
        r.notes[0].end_seconds = 0.31;
        assert!(score_note_intervals(&r, &g).is_err());
        assert!(
            vb::score_exported_notes(&r, &g).is_ok(),
            "Legacy performed-F0 diagnostic remains available"
        );
    }
    #[test]
    fn one_nanosecond_endpoint_roundoff_is_not_an_adjudication_failure() {
        let (mut r, mut g) = fixture();
        r.duration_seconds = 3.;
        g.duration_seconds = 3.;
        r.notes = vec![
            ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 2. + 1e-9,
                midi: 69,
            },
            ReferenceNote {
                start_seconds: 2.,
                end_seconds: 2.5,
                midi: 70,
            },
        ];
        set_notes(&mut g, &r.notes);
        let m = score_note_intervals(&r, &g).unwrap();
        assert_eq!(m.normalized_boundary_overlaps, 1);
        close(m.pitch_accuracy, 1.);
    }

    #[test]
    fn original_tempo_offset_and_playable_midi_round_trip_keep_note_time_credit() {
        let (mut r, mut g) = fixture();
        let map = TempoMap::new(
            480,
            0.03,
            vec![
                crate::timing::Tempo { tick: 0, bpm: 120. },
                crate::timing::Tempo {
                    tick: 480,
                    bpm: 240.,
                },
            ],
        )
        .unwrap();
        r.notes = vec![ReferenceNote {
            start_seconds: map.seconds(96),
            end_seconds: map.seconds(768),
            midi: 69,
        }];
        g.timing = Some(map.clone());
        set_notes(&mut g, &r.notes);
        close(score_note_intervals(&r, &g).unwrap().pitch_accuracy, 1.);
        // MIDI carries chart ticks/tempos; recording Offset belongs to the
        // original project and remains unchanged in the generation report.
        let mut midi_map = map.clone();
        midi_map.offset = 0.;
        let empty = crate::midi::new_midi(&midi_map, &Default::default()).unwrap();
        let bytes = vocals::append_midi(&empty, g.chart.as_ref().unwrap()).unwrap();
        g.chart = Some(vocals::import_midi(&bytes).unwrap());
        close(score_note_intervals(&r, &g).unwrap().pitch_accuracy, 1.);
        g.chart.as_mut().unwrap().notes[0].lyric = Syllable::Text {
            text: "equivocado".into(),
            join_next: false,
        };
        close(score_note_intervals(&r, &g).unwrap().pitch_accuracy, 1.);
    }
    #[test]
    fn source_clock_resolution_and_outside_recording_notes_are_not_hidden() {
        let (r, g) = fixture();
        let mut bad = g.clone();
        bad.audio_sha256 = "c".repeat(64);
        assert!(score_note_intervals(&r, &bad).is_err());
        let mut bad = g.clone();
        bad.duration_seconds = 1.1;
        assert!(score_note_intervals(&r, &bad).is_err());
        let mut bad = g.clone();
        bad.chart.as_mut().unwrap().resolution = 960;
        assert!(score_note_intervals(&r, &bad).is_err());
        let mut outside = g.clone();
        set_notes(
            &mut outside,
            &[ReferenceNote {
                start_seconds: 0.1,
                end_seconds: 1.1,
                midi: 69,
            }],
        );
        close(
            score_note_intervals(&r, &outside)
                .unwrap()
                .outside_recording_pitched_seconds,
            0.1,
        );
    }
}
