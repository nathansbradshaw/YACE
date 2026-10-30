//! Shared by the audit example and its saved-evidence regression test.
use anyhow::Result;
use chart_engine::mapping::{PitchNote, Transcription};

pub fn recovery_disabled(events: &Transcription) -> Result<Transcription> {
    events.validate()?;
    let mut baseline = events.clone();
    baseline.drums.retain(|hit| {
        !events.drum_recovered_hits.iter().any(|recovered| {
            recovered.time == hit.time
                && recovered.voice == hit.voice
                && recovered.confidence == hit.confidence
        })
    });
    baseline.drum_recovered_hits.clear();
    // This is a full recovery ablation, so disable articulation rather than
    // retaining proposals whose raw indices changed when hits were removed.
    baseline.drum_articulations.clear();
    baseline.drum_articulation_source = None;
    let same =
        |a: &PitchNote, b: &PitchNote| a.start == b.start && a.end == b.end && a.pitch == b.pitch;
    let recovered = |note: &PitchNote| {
        events
            .guitar_short_candidates
            .iter()
            .any(|candidate| candidate.retained && same(&candidate.evidence.candidate, note))
            || events
                .guitar_polyphonic_notes
                .iter()
                .any(|candidate| same(&candidate.evidence.candidate, note))
    };
    baseline.guitar.retain(|note| !recovered(note));
    baseline
        .guitar_candidates
        .retain(|candidate| !recovered(&candidate.candidate));
    baseline.guitar_short_candidates.clear();
    baseline.guitar_polyphonic_notes.clear();
    baseline.guitar_polyphonic_diagnostics.clear();
    baseline.warnings.retain(|warning| {
        ![
            "Short guitar recovery",
            "Polyphonic guitar recovery",
            "Dense-percussion recovery",
            "ADTOF's raw five-class hits and clocks",
        ]
        .iter()
        .any(|prefix| warning.starts_with(prefix))
    });
    baseline.validate()?;
    Ok(baseline)
}

pub fn pass_counts(events: &Transcription) -> serde_json::Value {
    serde_json::json!({
        "legacy_short_guitar_retained":events.guitar_short_candidates.iter().filter(|candidate|candidate.retained).count(),
        "polyphonic_guitar_retained":events.guitar_polyphonic_notes.len(),
        "additional_percussion_hits":events.drum_recovered_hits.len(),
        "drum_articulation_evidence":events.drum_articulations.len(),
        "drum_articulation_map_eligible":events.drum_articulations.iter().filter(|candidate|candidate.map_proposal).count(),
    })
}
