use chart_engine::{
    chart,
    mapping::{self, DrumHit, Transcription, Voice},
    models, review,
    review_regions::Reason,
    timing::TempoMap,
};

fn percussion(times: &[f64]) -> Vec<f32> {
    let mut audio = vec![0.; 22050 * 4];
    for &time in times {
        let start = (time * 22050.) as usize;
        for i in 0..1000 {
            let t = i as f32;
            audio[start + i] += 0.3 * (t * 1.37).sin() * (-t / 140.).exp();
        }
    }
    audio
}

#[test]
fn dense_syncopated_percussion_requires_audio_support_and_does_not_move_original_hits() {
    let times = [0.5, 0.875, 1.25, 1.5, 1.875, 2.25, 2.5, 2.875];
    let mut scores = vec![0.55; 400 * 5];
    for &time in &times {
        let frame = (time * 100.0_f64).round() as usize;
        scores[frame * 5 + 1] = 0.74;
    }
    // An ordinary high-contrast hit must stay exactly as the original decoder emitted it.
    scores[320 * 5 + 1] = 0.99;
    let baseline = models::drum_peaks(&scores);
    let (hits, recovered) = models::supported_drum_peaks(&scores, &percussion(&times));
    assert_eq!(recovered.len(), times.len());
    assert_eq!(hits.len(), baseline.len() + times.len());
    assert!(baseline.iter().all(|b| {
        hits.iter()
            .any(|h| h.time == b.time && h.voice == b.voice && h.confidence == b.confidence)
    }));
    for hit in &recovered {
        assert!(times.iter().any(|t| (t - hit.time).abs() <= 0.0051));
    }
    assert!(
        models::supported_drum_peaks(&scores, &vec![0.; 22050 * 4])
            .1
            .is_empty()
    );
    assert!(
        models::supported_drum_peaks(&vec![0.55; 400 * 5], &percussion(&times))
            .1
            .is_empty()
    );
}

#[test]
fn added_percussion_is_reviewable_only_when_a_generated_gem_is_played() {
    let hit = DrumHit {
        time: 0.875,
        voice: Voice::Snare,
        confidence: 0.74,
    };
    let events = Transcription {
        drums: vec![hit.clone()],
        drum_recovered_hits: vec![hit],
        ..Default::default()
    };
    let timing = TempoMap::constant(120.).unwrap();
    let tracks = mapping::tracks(&events, &timing, true).unwrap();
    let chart = chart::Chart::parse(chart::new_chart("Test", "Test", &timing, &tracks)).unwrap();
    let generated = ["ExpertDrums".into()].into_iter().collect();
    let map = review::generation_map(&chart, &events, 4., Default::default(), generated).unwrap();
    assert!(
        map.regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == Reason::RecoveredPercussionAttack)
    );
    let map = review::generation_map(&chart, &events, 4., Default::default(), Default::default())
        .unwrap();
    assert!(
        !map.regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == Reason::RecoveredPercussionAttack)
    );
    let mut changed = events.clone();
    changed.drums.clear();
    assert!(changed.validate().is_err());
}

#[test]
fn recovered_guitar_flags_require_the_selected_source_and_owned_audio_window() {
    use chart_engine::mapping::{
        PitchCandidateEvidence, PitchCandidateSource, PitchNote, ShortPitchCandidate,
    };
    let candidate = PitchCandidateEvidence {
        candidate: PitchNote {
            start: 0.5,
            end: 0.59,
            pitch: 64,
            confidence: 0.8,
        },
        source: PitchCandidateSource::NetworkOnset,
        onset_score: 0.9,
        inferred_onset_score: 0.9,
    };
    let receipt = ShortPitchCandidate {
        evidence: candidate.clone(),
        retained: true,
        reason: "source_supported".into(),
        source_attack_seconds: Some(0.5),
        periodic_pitch: Some(64.),
        periodicity: Some(0.99),
        fundamental_fraction: Some(0.8),
        window_start: Some(0.505),
        window_end: Some(0.55),
    };
    let events = Transcription {
        guitar: vec![candidate.candidate.clone()],
        guitar_candidates: vec![candidate],
        guitar_short_candidates: vec![receipt],
        ..Default::default()
    };
    let timing = TempoMap::constant(120.).unwrap();
    let mut tracks = mapping::tracks(&events, &timing, true).unwrap();
    let check = |tracks: &chart::Tracks| {
        let chart = chart::Chart::parse(chart::new_chart("Test", "Test", &timing, tracks)).unwrap();
        review::generation_map(
            &chart,
            &events,
            4.,
            Default::default(),
            ["ExpertSingle".into()].into_iter().collect(),
        )
        .unwrap()
        .regions
        .iter()
        .flat_map(|r| &r.issues)
        .filter(|i| i.reason == Reason::RecoveredShortNote)
        .count()
    };
    assert_eq!(check(&tracks), 1);
    tracks.get_mut("ExpertSingle").unwrap()[0].lane = 4;
    assert_eq!(check(&tracks), 0);
    let mut invalid = events.clone();
    invalid.guitar_short_candidates[0].window_end = Some(0.7);
    assert!(invalid.validate().is_err());
    let mut invalid = events;
    invalid.guitar_candidates.clear();
    assert!(invalid.validate().is_err());
}

#[test]
fn other_melody_chart_is_flagged_and_cannot_be_audited_against_guitar_audio() {
    use chart_engine::{
        mapping::{GuitarSource, PitchNote},
        stem_coverage,
        stem_review::StemId,
    };
    let events = Transcription {
        guitar_source: GuitarSource::Other,
        guitar: vec![PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 64,
            confidence: 0.8,
        }],
        ..Default::default()
    };
    assert_eq!(events.guitar_source.stem(), StemId::Other);
    assert_eq!(GuitarSource::Piano.stem(), StemId::Piano);
    assert_eq!(
        GuitarSource::Other.audition_stem("ExpertSingle", false),
        StemId::Other
    );
    assert_eq!(
        GuitarSource::Piano.audition_stem("HardSingle", false),
        StemId::Piano
    );
    assert_eq!(
        GuitarSource::Other.audition_stem("ExpertDoubleBass", false),
        StemId::Bass
    );
    assert_eq!(
        GuitarSource::Other.audition_stem("ExpertKeyboard", false),
        StemId::Piano
    );
    assert_eq!(
        GuitarSource::Other.audition_stem("ExpertSingle", true),
        StemId::Vocals
    );
    let timing = TempoMap::constant(120.).unwrap();
    let tracks = mapping::tracks(&events, &timing, true).unwrap();
    let chart = chart::Chart::parse(chart::new_chart("Test", "Test", &timing, &tracks)).unwrap();
    let map = review::generation_map(
        &chart,
        &events,
        4.,
        Default::default(),
        ["ExpertSingle".into()].into_iter().collect(),
    )
    .unwrap();
    assert!(
        map.regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == Reason::MelodySourceReview)
    );
    assert!(
        stem_coverage::analyze_stem(
            &chart,
            StemId::Guitar,
            &[],
            &[],
            &[],
            Some(&events),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn melody_audition_identity_survives_open_export_and_legacy_generation_evidence() {
    use chart_engine::{
        editor::{Action, Actor, ActorKind, Request, Store},
        mapping::{GuitarSource, PitchNote},
        pipeline::{self, Build},
    };
    use std::fs;
    let temp = tempfile::tempdir().unwrap();
    let audio = temp.path().join("audio.wav");
    models::write_audio(&audio, &vec![0.; 44100], &vec![0.; 44100]).unwrap();
    let mut meta = pipeline::metadata(&audio).unwrap();
    meta.name = "Routing".into();
    meta.artist = "Fixture".into();
    let events = Transcription {
        guitar_source: GuitarSource::Other,
        guitar: vec![PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 64,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    let song = temp.path().join("song");
    pipeline::build(Build {
        audio: &audio,
        output: &song,
        existing: None,
        timing: Some(TempoMap::constant(120.).unwrap()),
        transcription: &events,
        meta,
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    let store = Store::create(&song, None, &temp.path().join("project")).unwrap();
    let doc = store.load().unwrap();
    assert_eq!(store.guitar_source(&doc).unwrap(), GuitarSource::Other);
    // Earlier source-choice projects contain the bound transcription but no route sidecar.
    fs::remove_file(store.root.join("source/chart-engine-melody-source.json")).unwrap();
    assert_eq!(store.guitar_source(&doc).unwrap(), GuitarSource::Other);
    let source = store.root.join("source/chart-engine-transcription.json");
    let original = fs::read(&source).unwrap();
    fs::write(&source, br#"{"unrelated_legacy_metadata":true}"#).unwrap();
    assert_eq!(store.guitar_source(&doc).unwrap(), GuitarSource::Guitar);
    fs::write(&source, &original).unwrap();
    let mut changed = serde_json::from_slice::<serde_json::Value>(&original).unwrap();
    changed["guitar_source"] = serde_json::json!("piano");
    fs::write(&source, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(store.guitar_source(&doc).is_err());
    fs::write(&source, original).unwrap();
    let output = temp.path().join("export");
    store
        .apply(Request {
            expected_revision: doc.revision,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Routing fixture".into(),
            },
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let reopened = Store::create(&output, None, &temp.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.guitar_source(&reopened.load().unwrap()).unwrap(),
        GuitarSource::Other
    );
    let binding = reopened.root.join("source/chart-engine-melody-source.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding).unwrap()).unwrap();
    value["audio_sha256"] = serde_json::json!("0".repeat(64));
    fs::write(binding, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(reopened.guitar_source(&reopened.load().unwrap()).is_err());
}
