use chart_engine::{
    bass_analysis,
    mapping::{PitchCandidateEvidence, PitchCandidateSource, PitchNote, Transcription},
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn tone(path: &Path, frequency: f64) {
    let rate = 11025u32;
    let pcm: Vec<i16> = (0..rate * 2)
        .map(|i| {
            (0.3 * i16::MAX as f64
                * (std::f64::consts::TAU * frequency * i as f64 / rate as f64).sin())
                as i16
        })
        .collect();
    let length = pcm.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + length).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(length.to_le_bytes());
    for sample in pcm {
        bytes.extend(sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

fn raw() -> Transcription {
    let note = PitchNote {
        start: 0.2,
        end: 1.2,
        pitch: 57,
        confidence: 0.9,
    };
    Transcription {
        bass: vec![note.clone()],
        bass_candidates: vec![PitchCandidateEvidence {
            candidate: note,
            source: PitchCandidateSource::NetworkOnset,
            onset_score: 0.9,
            inferred_onset_score: 0.8,
        }],
        guitar: vec![PitchNote {
            start: 0.,
            end: 1.,
            pitch: 69,
            confidence: 0.8,
        }],
        warnings: vec!["Retain other-instrument diagnostics".into()],
        ..Default::default()
    }
}

#[test]
fn stale_analysis_replays_raw_evidence_without_compounding_corrections() {
    let directory = tempfile::tempdir().unwrap();
    let audio = directory.path().join("bass.wav");
    tone(&audio, 110.);
    let mut events = raw();
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert_eq!(events.bass[0].pitch, 45);
    assert_eq!(events.bass_corrections.len(), 1);
    assert!(bass_analysis::is_current(&events, &audio).unwrap());
    let first = serde_json::to_value(&events).unwrap();
    events.bass_analysis.as_mut().unwrap().implementation_sha256 = "0".repeat(64);
    assert!(!bass_analysis::is_current(&events, &audio).unwrap());
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert_eq!(serde_json::to_value(&events).unwrap(), first);
    assert_eq!(events.bass_corrections[0].original.pitch, 57);
    assert_eq!(
        events.bass_analysis.as_ref().unwrap().original_notes[0].pitch,
        57
    );
}

#[test]
fn changed_inputs_cannot_claim_current_analysis_and_failures_retain_events() {
    let directory = tempfile::tempdir().unwrap();
    let audio = directory.path().join("bass.wav");
    tone(&audio, 110.);
    let mut events = raw();
    bass_analysis::analyze(&mut events, &audio).unwrap();
    let saved = serde_json::to_vec(&events).unwrap();
    let mut edited = events.clone();
    edited.bass[0].pitch += 1;
    assert!(edited.validate().is_err());
    let mut edited = events.clone();
    edited.bass_candidates[0].onset_score = 0.1;
    assert!(edited.validate().is_err());
    assert!(bass_analysis::analyze(&mut events, &directory.path().join("missing.wav")).is_err());
    assert_eq!(serde_json::to_vec(&events).unwrap(), saved);
    tone(&audio, 130.81278265);
    assert!(!bass_analysis::is_current(&events, &audio).unwrap());
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert_eq!(
        events.bass[0].pitch, 57,
        "New source cannot retain an obsolete octave correction"
    );
    assert_eq!(events.guitar.len(), 1);
    assert!(
        events
            .warnings
            .contains(&"Retain other-instrument diagnostics".into())
    );
}

#[test]
fn legacy_and_model_free_events_keep_a_replayable_original() {
    let directory = tempfile::tempdir().unwrap();
    let audio = directory.path().join("bass.wav");
    tone(&audio, 110.);
    let mut events = raw();
    events.bass_candidates.clear();
    let json = serde_json::to_string(&events).unwrap();
    assert!(!json.contains("bass_analysis"));
    let mut events: Transcription = serde_json::from_str(&json).unwrap();
    events.validate().unwrap();
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert_eq!(events.bass[0].pitch, 45);
    let saved = serde_json::to_value(&events).unwrap();
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert_eq!(serde_json::to_value(&events).unwrap(), saved);
}

#[test]
fn decoder_receipts_cannot_be_replaced_by_refinement_replay() {
    let directory = tempfile::tempdir().unwrap();
    let audio = directory.path().join("bass.wav");
    tone(&audio, 110.);
    let mut events = raw();
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert!(bass_analysis::is_current(&events, &audio).unwrap());
    assert!(!bass_analysis::decoder_is_current(&events, &audio).unwrap());
    let digest = |value: &Vec<PitchCandidateEvidence>| {
        format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
    };
    events.bass_decoder = Some(bass_analysis::Decoder {
        version: 1,
        implementation_sha256: bass_analysis::decoder_implementation_hash(),
        model_sha256: chart_engine::models::assets()
            .into_iter()
            .find(|asset| asset.id == "pitch")
            .unwrap()
            .sha256,
        bass_audio_sha256: chart_engine::models::hash_file(&audio).unwrap(),
        candidates_sha256: digest(&events.bass_candidates),
        short_candidates_sha256: format!("{:x}", Sha256::digest(b"[]")),
    });
    assert!(bass_analysis::decoder_is_current(&events, &audio).unwrap());
    events.bass_decoder.as_mut().unwrap().implementation_sha256 = "0".repeat(64);
    bass_analysis::analyze(&mut events, &audio).unwrap();
    assert!(
        !bass_analysis::decoder_is_current(&events, &audio).unwrap(),
        "Replay cannot claim a new decoder ran"
    );
    let mut tampered = events.clone();
    tampered.bass_candidates[0].onset_score = 0.1;
    assert!(tampered.validate().is_err());
    let saved = serde_json::to_value(&events).unwrap();
    assert!(
        bass_analysis::infer(&mut events, &audio, &directory.path().join("missing.onnx")).is_err()
    );
    assert_eq!(
        serde_json::to_value(&events).unwrap(),
        saved,
        "Failed inference retains the previous analysis"
    );
}

#[test]
fn only_played_recovered_short_notes_receive_review_flags() {
    use chart_engine::{
        chart,
        mapping::{self, BassShortCandidate},
        review,
        review_regions::Reason,
        timing::TempoMap,
    };
    let mut events = raw();
    events.bass[0].end = 0.28;
    events.bass_candidates[0].candidate = events.bass[0].clone();
    events.bass_short_candidates = vec![BassShortCandidate {
        evidence: events.bass_candidates[0].clone(),
        retained: true,
        reason: "independent source attack and owned fundamental".into(),
        source_attack_seconds: Some(0.2),
        periodic_pitch: Some(57.),
        periodicity: Some(0.99),
        fundamental_fraction: Some(0.8),
        window_start: Some(0.2),
        window_end: Some(0.28),
    }];
    let timing = TempoMap::constant(120.).unwrap();
    let mut tracks = mapping::tracks(&events, &timing, false).unwrap();
    let check = |tracks: &chart::Tracks| {
        let chart = chart::Chart::parse(chart::new_chart("Test", "Test", &timing, tracks)).unwrap();
        review::generation_map(
            &chart,
            &events,
            2.,
            Default::default(),
            ["ExpertDoubleBass".into()].into_iter().collect(),
        )
        .unwrap()
        .regions
        .into_iter()
        .flat_map(|region| region.issues)
        .filter(|issue| issue.reason == Reason::RecoveredShortNote)
        .count()
    };
    assert_eq!(check(&tracks), 1);
    tracks.insert("ExpertDoubleBass".into(), vec![]);
    assert_eq!(
        check(&tracks),
        0,
        "Suppressed candidates must not create phantom flags"
    );
}

#[test]
fn retained_short_receipts_cannot_borrow_windows_or_claim_discarded_raw_notes() {
    use chart_engine::mapping::BassShortCandidate;
    let mut events = raw();
    let candidate = events.bass_candidates[0].clone();
    events.bass_short_candidates = vec![BassShortCandidate {
        evidence: candidate.clone(),
        retained: true,
        reason: "source_supported".into(),
        source_attack_seconds: Some(0.2),
        periodic_pitch: Some(57.),
        periodicity: Some(0.99),
        fundamental_fraction: Some(0.8),
        window_start: Some(0.21),
        window_end: Some(0.28),
    }];
    events.validate().unwrap();
    let valid = events.clone();
    events.bass_short_candidates[0].window_start = Some(0.19);
    assert!(
        events.validate().is_err(),
        "Previous note cannot supply source evidence"
    );
    events = valid.clone();
    events.bass_short_candidates[0].window_end = Some(1.21);
    assert!(
        events.validate().is_err(),
        "Following note cannot supply source evidence"
    );
    events = valid.clone();
    events.bass_candidates.clear();
    assert!(
        events.validate().is_err(),
        "A retained hypothesis must exist in the retained decoder output"
    );
    events = valid.clone();
    events.bass_candidates[0].onset_score = 0.5;
    assert!(
        events.validate().is_err(),
        "Same timing/pitch with different model evidence is not the original hypothesis"
    );
    events = valid;
    events.bass_short_candidates[0].retained = false;
    events.bass_candidates.clear();
    events.validate().unwrap();
}
