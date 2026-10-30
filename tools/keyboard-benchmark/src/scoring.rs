//! Reference scoring for raw detections. Matching is one-to-one and maximizes
//! cardinality; model confidence never supplies or changes reference labels.
//! Offset-inclusive matching is independent of the onset-only assignment.
use chart_engine::mapping::PitchNote;
use serde_json::{Value, json};
use std::{cmp::Ordering, collections::BTreeMap};

const ONSET_TOLERANCE: f64 = 0.050;
const OFFSET_MIN_TOLERANCE: f64 = 0.050;
const OFFSET_DURATION_FRACTION: f64 = 0.20;
// Protect exact threshold boundaries against binary floating-point subtraction.
// One nanosecond is far below every detector's source-clock resolution.
const BOUNDARY_EPSILON: f64 = 1e-9;

fn valid(note: &PitchNote) -> bool {
    note.pitch <= 127
        && note.start.is_finite()
        && note.end.is_finite()
        && note.start >= 0.
        && note.end > note.start
}

fn note_order(a: &PitchNote, b: &PitchNote) -> Ordering {
    a.start
        .total_cmp(&b.start)
        .then(a.pitch.cmp(&b.pitch))
        .then(a.end.total_cmp(&b.end))
}

fn assign(
    detection: usize,
    adjacency: &[Vec<usize>],
    seen: &mut [bool],
    owners: &mut [Option<usize>],
) -> bool {
    for &reference in &adjacency[detection] {
        if seen[reference] {
            continue;
        }
        seen[reference] = true;
        if owners[reference].is_none()
            || assign(owners[reference].unwrap(), adjacency, seen, owners)
        {
            owners[reference] = Some(detection);
            return true;
        }
    }
    false
}

/// Return detection owners indexed by reference note. Canonical iteration makes
/// matching/error summaries reproducible when equivalent inputs are reordered.
/// This is maximum-cardinality matching, not a claim of minimum total error.
fn matching(
    notes: &[PitchNote],
    reference: &[PitchNote],
    with_offsets: bool,
) -> Vec<Option<usize>> {
    let adjacency: Vec<Vec<usize>> = notes
        .iter()
        .map(|note| {
            let mut matches: Vec<_> = reference
                .iter()
                .enumerate()
                .filter_map(|(i, expected)| {
                    let onset_ok =
                        (note.start - expected.start).abs() <= ONSET_TOLERANCE + BOUNDARY_EPSILON;
                    let offset_tolerance = OFFSET_MIN_TOLERANCE
                        .max(OFFSET_DURATION_FRACTION * (expected.end - expected.start));
                    let offset_ok = !with_offsets
                        || (note.end - expected.end).abs() <= offset_tolerance + BOUNDARY_EPSILON;
                    (note.pitch == expected.pitch && onset_ok && offset_ok).then_some(i)
                })
                .collect();
            matches.sort_by(|&a, &b| {
                (reference[a].start - note.start)
                    .abs()
                    .total_cmp(&(reference[b].start - note.start).abs())
                    .then_with(|| {
                        (reference[a].end - note.end)
                            .abs()
                            .total_cmp(&(reference[b].end - note.end).abs())
                    })
                    .then_with(|| note_order(&reference[a], &reference[b]))
            });
            matches
        })
        .collect();
    let mut detections: Vec<_> = (0..notes.len()).collect();
    detections.sort_by(|&a, &b| note_order(&notes[a], &notes[b]));
    let mut owners = vec![None; reference.len()];
    for detection in detections {
        assign(
            detection,
            &adjacency,
            &mut vec![false; reference.len()],
            &mut owners,
        );
    }
    owners
}

fn quantile(values: &[f64], q: f64) -> Option<f64> {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    (!values.is_empty()).then(|| values[((values.len() - 1) as f64 * q).round() as usize])
}

fn rates(matched: usize, detections: usize, reference: usize) -> (f64, f64, f64) {
    let precision = if detections == 0 {
        if reference == 0 { 1. } else { 0. }
    } else {
        matched as f64 / detections as f64
    };
    let recall = if reference == 0 {
        1.
    } else {
        matched as f64 / reference as f64
    };
    let f1 = if precision + recall == 0. {
        0.
    } else {
        2. * precision * recall / (precision + recall)
    };
    (precision, recall, f1)
}

pub fn metrics(notes: &[PitchNote], reference: Option<&[PitchNote]>) -> Value {
    let mut last = BTreeMap::new();
    let mut rapid = Vec::new();
    let mut sorted: Vec<_> = notes.iter().filter(|note| valid(note)).collect();
    sorted.sort_by(|a, b| note_order(a, b));
    for note in sorted {
        if let Some(previous) = last.insert(note.pitch, note.start) {
            let gap = note.start - previous;
            if gap > 0. && gap < ONSET_TOLERANCE - BOUNDARY_EPSILON {
                rapid.push(json!({"pitch":note.pitch,"first":previous,"second":note.start,"gap_ms":gap*1000.}));
            }
        }
    }
    let mut result =
        json!({"notes":notes.len(),"rapid_same_key_under_50ms":rapid,"reference":null});
    let invalid_notes = notes.iter().filter(|note| !valid(note)).count();
    let invalid_reference =
        reference.map_or(0, |notes| notes.iter().filter(|note| !valid(note)).count());
    if invalid_notes > 0 || invalid_reference > 0 {
        result["scoring_error"] = json!({"reason":"Invalid note pitch or source-clock interval","invalid_notes":invalid_notes,"invalid_reference_notes":invalid_reference});
        return result;
    }
    let Some(reference) = reference else {
        return result;
    };
    let owners = matching(notes, reference, false);
    let offset_owners = matching(notes, reference, true);
    let matched = owners.iter().flatten().count();
    let offset_matched = offset_owners.iter().flatten().count();
    let (precision, recall, f1) = rates(matched, notes.len(), reference.len());
    let (offset_precision, offset_recall, offset_f1) =
        rates(offset_matched, notes.len(), reference.len());
    let errors: Vec<_> = owners
        .iter()
        .enumerate()
        .filter_map(|(j, i)| i.map(|i| (notes[i].start - reference[j].start) * 1000.))
        .collect();
    let onset_absolute: Vec<_> = errors.iter().map(|error| error.abs()).collect();
    // Include bad releases here: conditioning on offset-correct pairs would
    // hide the very long/short holds that the offset-inclusive score exposes.
    let keyoff_absolute: Vec<_> = owners
        .iter()
        .enumerate()
        .filter_map(|(j, i)| i.map(|i| (notes[i].end - reference[j].end).abs() * 1000.))
        .collect();
    result["reference"] = json!({
        "reference_notes":reference.len(),"matched_exact_pitch_onset_50ms":matched,
        "precision":precision,"recall":recall,"f1":f1,
        "onset_signed_ms_p50":quantile(&errors,0.5),"onset_absolute_ms_p95":quantile(&onset_absolute,0.95),
        "false_positives":notes.len()-matched,"misses":reference.len()-matched,
        "matched_exact_pitch_onset_offset":offset_matched,
        "offset_precision":offset_precision,"offset_recall":offset_recall,"offset_f1":offset_f1,
        "offset_false_positives":notes.len()-offset_matched,"offset_misses":reference.len()-offset_matched,
        "keyoff_absolute_ms_p50":quantile(&keyoff_absolute,0.5),"keyoff_absolute_ms_p95":quantile(&keyoff_absolute,0.95),"keyoff_absolute_ms_max":quantile(&keyoff_absolute,1.0),
        "keyoff_error_pairing":"canonical onset-only maximum-cardinality matching; not minimum total error",
        "onset_tolerance_ms":ONSET_TOLERANCE*1000.,"offset_minimum_tolerance_ms":OFFSET_MIN_TOLERANCE*1000.,"offset_reference_duration_fraction":OFFSET_DURATION_FRACTION
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn note(start: f64, end: f64, pitch: u8) -> PitchNote {
        PitchNote {
            start,
            end,
            pitch,
            confidence: 1.,
        }
    }
    #[test]
    fn duplicates_cannot_reuse_one_reference_note() {
        let score = metrics(
            &[note(0., 0.3, 60), note(0.01, 0.3, 60)],
            Some(&[note(0., 0.3, 60)]),
        );
        assert_eq!(score["notes"], 2);
        assert_eq!(score["reference"]["matched_exact_pitch_onset_50ms"], 1);
        assert_eq!(score["reference"]["matched_exact_pitch_onset_offset"], 1);
        assert_eq!(score["reference"]["false_positives"], 1);
        assert_eq!(score["reference"]["offset_false_positives"], 1);
    }
    #[test]
    fn competing_matches_use_augmenting_paths_and_are_order_invariant() {
        let notes = [note(0.04, 0.34, 60), note(0., 0.3, 60)];
        let reference = [note(0., 0.3, 60), note(0.08, 0.38, 60)];
        let score = metrics(&notes, Some(&reference));
        assert_eq!(score["reference"]["matched_exact_pitch_onset_50ms"], 2);
        assert_eq!(score["reference"]["matched_exact_pitch_onset_offset"], 2);
        assert_eq!(score["reference"]["f1"], 1.);
        assert_eq!(
            score,
            metrics(
                &[notes[1].clone(), notes[0].clone()],
                Some(&[reference[1].clone(), reference[0].clone()])
            )
        );
    }
    #[test]
    fn wrong_octave_and_wrong_holds_do_not_receive_offset_credit() {
        let reference = [note(0., 0.3, 60)];
        let octave = metrics(&[note(0., 0.3, 72)], Some(&reference));
        assert_eq!(octave["reference"]["f1"], 0.);
        assert_eq!(octave["reference"]["offset_f1"], 0.);
        let hold = metrics(&[note(0., 5., 60)], Some(&reference));
        assert_eq!(hold["reference"]["f1"], 1.);
        assert_eq!(hold["reference"]["offset_f1"], 0.);
        assert_eq!(hold["reference"]["keyoff_absolute_ms_max"], 4700.);
        assert_eq!(hold["reference"]["offset_false_positives"], 1);
        assert_eq!(hold["reference"]["offset_misses"], 1);
    }
    #[test]
    fn offset_matching_can_choose_a_different_valid_assignment() {
        let score = metrics(
            &[note(0., 1., 60), note(0.04, 0.3, 60)],
            Some(&[note(0., 0.3, 60), note(0.04, 1., 60)]),
        );
        assert_eq!(score["reference"]["f1"], 1.);
        assert_eq!(score["reference"]["offset_f1"], 1.);
        assert_eq!(score["reference"]["matched_exact_pitch_onset_offset"], 2);
        assert_eq!(score["reference"]["keyoff_absolute_ms_max"], 700.);
    }
    #[test]
    fn offsets_allow_the_larger_of_fifty_ms_or_twenty_percent_reference_duration() {
        let reference = [note(0., 1., 60), note(2., 2.1, 62)];
        let within = metrics(&[note(0., 1.2, 60), note(2., 2.15, 62)], Some(&reference));
        assert_eq!(within["reference"]["offset_f1"], 1.);
        let outside = metrics(
            &[note(0., 1.20001, 60), note(2., 2.15001, 62)],
            Some(&reference),
        );
        assert_eq!(outside["reference"]["offset_f1"], 0.);
    }
    #[test]
    fn empty_negative_controls_and_unannotated_real_audio_remain_distinct() {
        let empty = metrics(&[], Some(&[]));
        assert_eq!(empty["reference"]["f1"], 1.);
        assert_eq!(empty["reference"]["offset_f1"], 1.);
        assert!(empty["reference"]["keyoff_absolute_ms_p95"].is_null());
        let hallucination = metrics(&[note(0., 0.3, 60)], Some(&[]));
        assert_eq!(hallucination["reference"]["f1"], 0.);
        assert_eq!(hallucination["reference"]["false_positives"], 1);
        let missed = metrics(&[], Some(&[note(0., 0.3, 60)]));
        assert_eq!(missed["reference"]["offset_f1"], 0.);
        assert!(metrics(&[note(0., 0.3, 60)], None)["reference"].is_null());
    }
    #[test]
    fn fifty_ms_repeats_keep_every_note_and_only_faster_repeats_are_flagged() {
        let notes: Vec<_> = [0., 0.05, 0.10, 0.15]
            .into_iter()
            .map(|t| note(t, t + 0.03, 60))
            .collect();
        let score = metrics(&notes, Some(&notes));
        assert_eq!(score["notes"], 4);
        assert_eq!(score["reference"]["matched_exact_pitch_onset_offset"], 4);
        assert_eq!(score["rapid_same_key_under_50ms"], json!([]));
        let fast = [note(0., 0.02, 60), note(0.049, 0.07, 60)];
        let score = metrics(&fast, Some(&fast));
        assert_eq!(score["notes"], 2);
        assert_eq!(
            score["rapid_same_key_under_50ms"].as_array().unwrap().len(),
            1
        );
    }
    #[test]
    fn invalid_intervals_never_receive_fabricated_scores() {
        let invalid = metrics(&[note(f64::NAN, 1., 60)], Some(&[note(0., 1., 60)]));
        assert!(invalid["reference"].is_null());
        assert_eq!(invalid["scoring_error"]["invalid_notes"], 1);
        let invalid = metrics(&[], Some(&[note(1., 0., 60)]));
        assert_eq!(invalid["scoring_error"]["invalid_reference_notes"], 1);
    }
    #[test]
    fn maximum_matching_agrees_with_independent_exhaustive_pair_enumeration() {
        fn brute(
            notes: &[PitchNote],
            reference: &[PitchNote],
            index: usize,
            used: u32,
            offsets: bool,
        ) -> usize {
            if index == notes.len() {
                return 0;
            }
            let mut best = brute(notes, reference, index + 1, used, offsets);
            for (j, expected) in reference.iter().enumerate() {
                let candidate = &notes[index];
                let tolerance = 0.05f64.max(0.2 * (expected.end - expected.start));
                if used & (1 << j) == 0
                    && candidate.pitch == expected.pitch
                    && (candidate.start - expected.start).abs() <= 0.05 + BOUNDARY_EPSILON
                    && (!offsets
                        || (candidate.end - expected.end).abs() <= tolerance + BOUNDARY_EPSILON)
                {
                    best =
                        best.max(1 + brute(notes, reference, index + 1, used | (1 << j), offsets));
                }
            }
            best
        }
        let candidates = [
            note(0., 0.2, 60),
            note(0.04, 0.24, 60),
            note(0.08, 0.4, 60),
            note(0.04, 0.34, 62),
        ];
        let references = [
            note(0., 0.2, 60),
            note(0.08, 0.28, 60),
            note(0.04, 0.24, 62),
        ];
        for subset in 0u32..1 << candidates.len() {
            let notes: Vec<_> = candidates
                .iter()
                .enumerate()
                .filter(|(i, _)| subset & (1 << i) != 0)
                .map(|(_, n)| n.clone())
                .collect();
            for selected in 0u32..1 << references.len() {
                let reference: Vec<_> = references
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| selected & (1 << i) != 0)
                    .map(|(_, n)| n.clone())
                    .collect();
                for offsets in [false, true] {
                    assert_eq!(
                        matching(&notes, &reference, offsets)
                            .iter()
                            .flatten()
                            .count(),
                        brute(&notes, &reference, 0, 0, offsets),
                        "subsets {subset}/{selected}, offsets {offsets}"
                    );
                }
            }
        }
    }
}
