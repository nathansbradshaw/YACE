//! Acoustic decisions shared by native and browser generation. No transcript
//! enters pitch selection, segmentation, or alternative scoring.
use super::*;

pub(super) fn recover_alternatives(
    frames: &mut [PitchFrame],
    signal: &[f32],
    gate: f64,
    issues: &mut Vec<Issue>,
) {
    let mut pending = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        if frame.midi.is_some()
            || frame.local_energy_active == Some(false)
            || frame.local_rms.unwrap_or(0.) < gate
        {
            continue;
        }
        let mut candidates = Vec::new();
        for (detector, pitch) in [frame.yin_midi, frame.mcleod_midi].into_iter().enumerate() {
            let Some(pitch) = pitch.filter(|p| (35.5..84.5).contains(p)) else {
                continue;
            };
            let stable = frames[index.saturating_sub(2)..(index + 3).min(frames.len())]
                .iter()
                .filter(|f| {
                    let raw = if detector == 0 {
                        f.yin_midi
                    } else {
                        f.mcleod_midi
                    };
                    f.local_rms.unwrap_or(0.) >= gate
                        && raw.is_some_and(|p| (p - pitch).abs() <= 0.5)
                })
                .count();
            let center = (frame.time_seconds * RATE as f64).round() as usize;
            let correlation = period_support(signal, center, pitch);
            let harmonic = [12., 19.01955000865, 24.]
                .into_iter()
                .map(|delta| period_support(signal, center, pitch + delta))
                .fold(-1., f64::max);
            if stable >= 4 && correlation >= 0.85 && correlation - harmonic >= 0.15 {
                candidates.push((pitch, correlation));
            }
        }
        // Stable, source-supported evidence is necessary even for flagged notes.
        // Deterministic ties preserve detector order; this does not identify a singer.
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Some(&(pitch, correlation)) = candidates.first() {
            pending.push((index, pitch, correlation));
        }
    }
    for (index, pitch, correlation) in pending {
        let frame = &mut frames[index];
        frame.midi = Some(pitch);
        frame.support = PitchSupport::AcousticAlternative;
        frame.clarity = correlation * 0.5;
        issue(
            issues,
            frame.time_seconds,
            frame.time_seconds + HOP as f64 / RATE as f64,
            Reason::PitchDisagreement,
            "Experimental acoustic proposal: stable local detector contour and fundamental-period support; detector disagreement or dropout remains unresolved. Retained as a flagged draft note, not an approved lead pitch.",
        );
    }
}

pub(super) fn note_evidence(
    chart: Option<&VocalChart>,
    frames: &[PitchFrame],
    timing: &TempoMap,
) -> Vec<NoteEvidence> {
    chart.into_iter().flat_map(|chart| &chart.notes).filter_map(|note| {
        let VocalTone::Pitched { midi } = note.tone else { return None };
        let mut alternatives = Vec::new();
        let mut support = Vec::new();
        let mut uncertain = false;
        let start = timing.seconds(note.start);
        let end = timing.seconds(note.end);
        let first = frames.partition_point(|f| f.time_seconds < start);
        let last = frames.partition_point(|f| f.time_seconds < end);
        for frame in &frames[first..last] {
            if frame.midi.is_none() { continue; }
            if !support.contains(&frame.support) { support.push(frame.support); }
            uncertain |= frame.support != PitchSupport::Consensus || frame.clarity < 0.6;
            for raw in [frame.yin_midi, frame.mcleod_midi].into_iter().flatten() {
                if (35.5..84.5).contains(&raw) && (raw - midi as f64).abs() > 0.7 {
                    let alternative = raw.round() as u8;
                    if alternative != midi && !alternatives.contains(&alternative) { alternatives.push(alternative); }
                }
            }
        }
        alternatives.sort_unstable();
        uncertain |= !alternatives.is_empty();
        let reasons = if uncertain { vec!["Recovered, low-clarity or conflicting acoustic evidence; compare the recording and alternative pitches.".into()] } else { vec![] };
        Some(NoteEvidence { start: note.start, end: note.end, selected_midi: midi, alternatives, support, uncertain, reasons })
    }).collect()
}

/// Attach a word only when it has a unique largest overlap. Note boundaries
/// are already fixed. Unattached words remain available in the report.
pub(super) fn attach_words(
    chart: &mut VocalChart,
    words: Option<&WordTimings>,
    timing: &TempoMap,
    issues: &mut Vec<Issue>,
) {
    let Some(words) = words else {
        return;
    };
    let mut previous_owner = None;
    for index in 0..chart.notes.len() {
        let start = timing.seconds(chart.notes[index].start);
        let end = timing.seconds(chart.notes[index].end);
        let mut overlaps: Vec<_> = words
            .words
            .iter()
            .enumerate()
            .filter_map(|(i, word)| {
                let (a, b) = word.start.zip(word.end)?;
                let text = word.text.as_ref().filter(|s| !s.trim().is_empty())?;
                let overlap = end.min(b) - start.max(a);
                (overlap >= 0.030).then_some((i, overlap, text))
            })
            .collect();
        overlaps.sort_by(|a, b| b.1.total_cmp(&a.1));
        if overlaps.len() > 1 {
            issue(
                issues,
                start,
                end,
                Reason::WordLevelAlignment,
                "Multiple words intersect one acoustic note. Note boundaries were preserved; review lyric ownership and articulation. All recognized words remain in the generation report.",
            );
        }
        let owner = overlaps
            .first()
            .filter(|best| overlaps.get(1).is_none_or(|next| best.1 - next.1 > 0.010));
        if let Some((owner, _, text)) = owner {
            let same_phrase = index > 0
                && chart.phrases.iter().any(|p| {
                    p.start <= chart.notes[index - 1].start && chart.notes[index].end <= p.end
                });
            chart.notes[index].lyric = if previous_owner == Some(*owner) && same_phrase {
                if chart.notes[index]
                    .start
                    .saturating_sub(chart.notes[index - 1].end)
                    <= 1
                {
                    Syllable::Continue
                } else {
                    Syllable::ContinueWord
                }
            } else {
                Syllable::Text {
                    text: (*text).clone(),
                    join_next: false,
                }
            };
            previous_owner = Some(*owner);
        } else {
            previous_owner = None;
        }
    }
}
