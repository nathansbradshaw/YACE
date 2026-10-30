#[path = "../examples/support/rhythm_audit.rs"]
mod recovery;

use chart_engine::{
    drum_articulation::{Evidence, Source},
    mapping::{
        DrumHit, GuitarPolyphonicNote, PitchCandidateEvidence, PitchCandidateSource, PitchNote,
        ShortPitchCandidate, Transcription, Voice,
    },
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
};

fn candidate(
    start: f64,
    end: f64,
    pitch: u8,
    confidence: f32,
    onset: f32,
) -> PitchCandidateEvidence {
    PitchCandidateEvidence {
        candidate: PitchNote {
            start,
            end,
            pitch,
            confidence,
        },
        source: PitchCandidateSource::NetworkOnset,
        onset_score: onset,
        inferred_onset_score: onset,
    }
}
fn fixture(directory: &std::path::Path) -> Transcription {
    let core = candidate(0.25, 0.8, 52, 0.8, 0.9);
    let legacy = candidate(1.25, 1.35, 55, 0.8, 0.9);
    let additional = candidate(2.25, 2.35, 59, 0.4, 0.65);
    let bass = candidate(0.5, 1.5, 40, 0.8, 0.9);
    let manifest=StemManifest{version:1,source_audio_sha256:"a".repeat(64),sample_rate:44100,channels:2,frames:4*44100,
        alignment:AlignmentEvidence{method:"controlled saved-receipt schema fixture".into(),details:"Synthetic identity/clock metadata only; no acoustic inference or musical correctness is claimed".into()},
        stems:vec![StemAsset{id:StemId::Drums,path:directory.join("synthetic-drums.wav"),sha256:"b".repeat(64)}],generation:None};
    let hits = vec![
        DrumHit {
            time: 0.5,
            voice: Voice::Cymbal,
            confidence: 0.9,
        },
        DrumHit {
            time: 1.,
            voice: Voice::Tom,
            confidence: 0.8,
        },
        DrumHit {
            time: 1.5,
            voice: Voice::Snare,
            confidence: 0.95,
        },
    ];
    let articulation = |index: usize, proposed: Voice| Evidence {
        hit_index: index,
        time: hits[index].time,
        original: hits[index].voice,
        original_confidence: hits[index].confidence,
        proposed: Some(proposed),
        map_proposal: true,
        cymbal_model: None,
        confidence: 0.7,
        reason: "controlled_test_only".into(),
        resonance_hz: None,
        resonance_spread_semitones: None,
        spectral_peak_fraction: None,
        spectral_flatness: None,
        window_start: Some(hits[index].time + 0.003),
        window_end: Some(hits[index].time + 0.05),
        window_samples: Some(512),
    };
    Transcription {
        guitar: vec![
            core.candidate.clone(),
            legacy.candidate.clone(),
            additional.candidate.clone(),
        ],
        guitar_candidates: vec![core, legacy.clone(), additional.clone()],
        guitar_short_candidates: vec![
            ShortPitchCandidate {
                evidence: legacy,
                retained: true,
                reason: "source_supported".into(),
                source_attack_seconds: Some(1.25),
                periodic_pitch: Some(55.),
                periodicity: Some(0.99),
                fundamental_fraction: Some(0.8),
                window_start: Some(1.253),
                window_end: Some(1.32),
            },
            ShortPitchCandidate {
                evidence: additional.clone(),
                retained: false,
                reason: "network_support_not_strong".into(),
                source_attack_seconds: None,
                periodic_pitch: None,
                periodicity: None,
                fundamental_fraction: None,
                window_start: None,
                window_end: None,
            },
        ],
        guitar_polyphonic_notes: vec![GuitarPolyphonicNote {
            evidence: additional,
            original_rejection: "network_support_not_strong".into(),
            source_attack_seconds: 2.25,
            window_start: 2.253,
            window_end: 2.34,
            minimum_fundamental_power: 0.01,
            minimum_target_variance_gain: 0.2,
            minimum_source_explained: 0.6,
            maximum_subharmonic_power: 0.0001,
            frequency_offset_hz: 0.,
            preexisting_power_fraction: None,
        }],
        guitar_polyphonic_diagnostics: std::collections::BTreeMap::from([(
            "supported_joint_harmonics".into(),
            1,
        )]),
        drum_articulations: vec![
            articulation(0, Voice::Ride),
            articulation(1, Voice::HighTom),
        ],
        drum_articulation_source: Some(Source::from_manifest(&manifest).unwrap()),
        drum_recovered_hits: vec![hits[1].clone()],
        drums: hits,
        isolation: Some(manifest),
        bass: vec![bass.candidate.clone()],
        bass_candidates: vec![bass],
        warnings: vec![
            "Preserve unrelated evidence".into(),
            "Short guitar recovery test".into(),
            "Polyphonic guitar recovery test".into(),
            "Dense-percussion recovery test".into(),
            "ADTOF's raw five-class hits and clocks test".into(),
        ],
        ..Default::default()
    }
}

#[test]
fn saved_typed_receipts_disable_all_passes_without_losing_core_notes_or_binding() {
    let directory = tempfile::tempdir().unwrap();
    let events = fixture(directory.path());
    events.validate().unwrap();
    let path = directory.path().join("events.json");
    let saved = serde_json::to_vec_pretty(&events).unwrap();
    std::fs::write(&path, &saved).unwrap();
    let events: Transcription = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    // Reproduce both old clone defects independently before exercising the fix.
    let mut broken = events.clone();
    broken.guitar_short_candidates.clear();
    assert!(broken.validate().is_err());
    let mut broken = events.clone();
    broken.drums.remove(1);
    broken.drum_recovered_hits.clear();
    assert!(broken.validate().is_err());
    let baseline = recovery::recovery_disabled(&events).unwrap();
    baseline.validate().unwrap();
    assert_eq!(serde_json::to_vec_pretty(&events).unwrap(), saved);
    assert_eq!(
        serde_json::to_value(&baseline.bass).unwrap(),
        serde_json::to_value(&events.bass).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&baseline.bass_candidates).unwrap(),
        serde_json::to_value(&events.bass_candidates).unwrap()
    );
    assert_eq!(baseline.isolation, events.isolation);
    assert_eq!(baseline.guitar.len(), 1);
    assert_eq!(
        serde_json::to_value(&baseline.guitar[0]).unwrap(),
        serde_json::to_value(&events.guitar[0]).unwrap()
    );
    assert_eq!(baseline.guitar_candidates.len(), 1);
    assert_eq!(
        serde_json::to_value(&baseline.drums).unwrap(),
        serde_json::to_value(vec![events.drums[0].clone(), events.drums[2].clone()]).unwrap()
    );
    assert!(
        baseline.guitar_short_candidates.is_empty()
            && baseline.guitar_polyphonic_notes.is_empty()
            && baseline.guitar_polyphonic_diagnostics.is_empty()
    );
    assert!(
        baseline.drum_recovered_hits.is_empty()
            && baseline.drum_articulations.is_empty()
            && baseline.drum_articulation_source.is_none()
    );
    assert_eq!(baseline.warnings, vec!["Preserve unrelated evidence"]);
    assert_eq!(
        recovery::pass_counts(&events),
        serde_json::json!({"legacy_short_guitar_retained":1,"polyphonic_guitar_retained":1,"additional_percussion_hits":1,"drum_articulation_evidence":2,"drum_articulation_map_eligible":2})
    );
    assert_eq!(
        recovery::pass_counts(&baseline),
        serde_json::json!({"legacy_short_guitar_retained":0,"polyphonic_guitar_retained":0,"additional_percussion_hits":0,"drum_articulation_evidence":0,"drum_articulation_map_eligible":0})
    );
}

#[test]
fn ablation_cannot_hide_invalid_original_source_binding() {
    let directory = tempfile::tempdir().unwrap();
    let mut events = fixture(directory.path());
    events
        .drum_articulation_source
        .as_mut()
        .unwrap()
        .stem_sha256 = "c".repeat(64);
    assert!(events.validate().is_err());
    assert!(recovery::recovery_disabled(&events).is_err());
}
