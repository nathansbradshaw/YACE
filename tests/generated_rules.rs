use chart_engine::{
    chart::{Note, Tracks},
    generated_rules::{RuleCode, Severity, assess},
    mapping::{self, DrumHit, PitchNote, Transcription, Voice},
    timing::{Tempo, TempoMap},
};

fn note(tick: u64, lane: u8, sustain: u64) -> Note {
    Note {
        tick,
        lane,
        sustain,
    }
}
fn tracks(track: &str, notes: Vec<Note>) -> Tracks {
    Tracks::from([(track.into(), notes)])
}

#[test]
fn structural_failures_are_errors_and_reports_are_deterministic() {
    let timing = TempoMap::constant(120.0).unwrap();
    let mut input = tracks(
        "ExpertDrums",
        vec![
            note(0, 1, 0),
            note(0, 1, 0),
            note(100, 66, 0),
            note(200, 65, 0),
            note(300, 0, 1),
            note(400, 68, 1),
        ],
    );
    input.insert(
        "ExpertSingle".into(),
        vec![
            note(0, 7, 0),
            note(0, 0, 0),
            note(200, 8, 0),
            note(u64::MAX, 1, 1),
            note(4800, 2, 0),
        ],
    );
    let report = assess(&input, &timing, 4.0);
    assert!(report.has_errors());
    for code in [
        RuleCode::DuplicateLane,
        RuleCode::OrphanCymbalMarker,
        RuleCode::UnsupportedLane,
        RuleCode::DrumSustain,
        RuleCode::ControlMarkerSustain,
        RuleCode::OpenColoredChord,
        RuleCode::TickOverflow,
        RuleCode::BeyondRecording,
    ] {
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == code && finding.severity == Severity::Error),
            "Missing {code:?}"
        );
    }
    for notes in input.values_mut() {
        notes.reverse();
    }
    assert_eq!(report, assess(&input, &timing, 4.0));
    assert_eq!(report.musical_acceptance, "unverified");
}

#[test]
fn plausible_fast_patterns_and_sustain_overlaps_request_review_only() {
    let timing = TempoMap::constant(120.0).unwrap();
    let mut input = tracks(
        "ExpertDrums",
        vec![
            note(480, 1, 0),
            note(486, 2, 0),
            note(492, 3, 0),
            note(500, 1, 0),
        ],
    );
    input.insert(
        "ExpertSingle".into(),
        vec![note(480, 0, 100), note(500, 0, 0)],
    );
    input.insert("HardSingle".into(), vec![note(490, 0, 0)]);
    let report = assess(&input, &timing, 4.0);
    assert!(!report.has_errors());
    for code in [
        RuleCode::FastRepeat,
        RuleCode::HandCrowding,
        RuleCode::SustainRetriggerOverlap,
        RuleCode::LowerTierAttackAbsentFromExpert,
    ] {
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == code && finding.severity == Severity::Review),
            "Missing {code:?}"
        );
    }
}

#[test]
fn valid_generated_profile_preserves_controls_tempo_offsets_and_boundaries() {
    let timing = TempoMap::new(
        480,
        -0.25,
        vec![
            Tempo {
                tick: 0,
                bpm: 120.0,
            },
            Tempo {
                tick: 960,
                bpm: 60.0,
            },
        ],
    )
    .unwrap();
    let mut input = tracks(
        "ExpertDrums",
        vec![
            note(0, 0, 0),
            note(480, 2, 0),
            note(480, 66, 0),
            note(480, 35, 0),
            note(480, 41, 0),
            note(960, 32, 0),
        ],
    );
    input.insert(
        "ExpertSingle".into(),
        vec![
            note(480, 7, 0),
            note(480, 5, 0),
            note(480, 6, 0),
            note(2040, 1, 0),
        ],
    );
    // Last note is 3s after an offset/tempo change, inside the 100ms allowance.
    let duration = timing.seconds(2040) - 0.05;
    let report = assess(&input, &timing, duration);
    assert!(report.findings.is_empty(), "{report:?}");
    let report = assess(&input, &timing, duration - 0.1);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == RuleCode::BeyondRecording
                && (f.seconds - timing.seconds(2040)).abs() < 1e-9)
    );
}

#[test]
fn current_mapper_outputs_pass_structural_gate_without_alteration() {
    let input = Transcription {
        drums: vec![
            DrumHit {
                time: 0.5,
                voice: Voice::Kick,
                confidence: 0.9,
            },
            DrumHit {
                time: 0.5,
                voice: Voice::HiHat,
                confidence: 0.9,
            },
            DrumHit {
                time: 1.0,
                voice: Voice::Snare,
                confidence: 0.9,
            },
        ],
        guitar: vec![
            PitchNote {
                start: 0.5,
                end: 0.8,
                pitch: 52,
                confidence: 0.9,
            },
            PitchNote {
                start: 1.0,
                end: 1.4,
                pitch: 60,
                confidence: 0.9,
            },
        ],
        bass: vec![PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 36,
            confidence: 0.9,
        }],
        ..Transcription::default()
    };
    let timing = TempoMap::constant(120.0).unwrap();
    let generated = mapping::tracks(&input, &timing, true).unwrap();
    let unchanged = generated.clone();
    assert!(!assess(&generated, &timing, 2.0).has_errors());
    assert_eq!(generated, unchanged);
}

#[test]
fn invalid_context_and_orphan_techniques_are_explicit_errors() {
    let input = tracks("ExpertSingle", vec![note(0, 5, 0)]);
    let timing = TempoMap::constant(120.0).unwrap();
    assert!(
        assess(&input, &timing, 1.0)
            .findings
            .iter()
            .any(|f| f.code == RuleCode::OrphanControlMarker)
    );
    let mut invalid = timing.clone();
    invalid.resolution = 0;
    let report = assess(&input, &invalid, f64::NAN);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == RuleCode::InvalidTiming)
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == RuleCode::InvalidDuration)
    );
    serde_json::to_string(&report).unwrap();
}

#[test]
fn generated_soft_rules_reach_timed_map_without_duplicate_pattern_flags() {
    use chart_engine::{chart, pipeline, review, review_regions::Reason};
    use std::collections::{BTreeMap, BTreeSet};
    let timing = TempoMap::new(
        480,
        0.25,
        vec![Tempo {
            tick: 0,
            bpm: 120.0,
        }],
    )
    .unwrap();
    let mut generated = tracks(
        "ExpertDrums",
        vec![
            note(480, 1, 0),
            note(486, 2, 0),
            note(492, 3, 0),
            note(500, 1, 0),
        ],
    );
    generated.insert(
        "ExpertSingle".into(),
        vec![note(480, 0, 100), note(500, 0, 0)],
    );
    generated.insert("HardSingle".into(), vec![note(490, 0, 0)]);
    let chart =
        chart::Chart::parse(chart::new_chart("Test", "Artist", &timing, &generated)).unwrap();
    let map = review::generation_map(
        &chart,
        &Transcription::default(),
        4.0,
        BTreeMap::new(),
        generated.keys().cloned().collect(),
    )
    .unwrap();
    let original_fast: BTreeSet<_> = map
        .regions
        .iter()
        .flat_map(|region| &region.issues)
        .filter(|issue| issue.reason == Reason::FastRepeat)
        .map(|issue| (issue.track.clone(), issue.start.to_bits()))
        .collect();
    let rules = assess(&generated, &timing, 4.0);
    let merged = pipeline::with_generated_rule_reviews(map, &rules).unwrap();
    let issues: Vec<_> = merged
        .regions
        .iter()
        .flat_map(|region| &region.issues)
        .collect();
    for reason in [
        Reason::SustainOverlap,
        Reason::ReductionDifference,
        Reason::HandCrowding,
    ] {
        assert!(
            issues.iter().any(|issue| issue.reason == reason),
            "Missing {reason:?}"
        );
    }
    let fast: Vec<_> = issues
        .iter()
        .filter(|issue| issue.reason == Reason::FastRepeat)
        .collect();
    assert_eq!(fast.len(), original_fast.len());
    assert_eq!(
        issues
            .iter()
            .filter(|issue| issue.reason == Reason::HandCrowding)
            .count(),
        1
    );
    let overlap = issues
        .iter()
        .find(|issue| issue.reason == Reason::SustainOverlap)
        .unwrap();
    assert_eq!(overlap.track, "ExpertSingle");
    assert!((overlap.start - timing.seconds(500)).abs() < 1e-9);
    assert_eq!(overlap.evidence["generated_rule_tick"], 500.0);
    let reduction = issues
        .iter()
        .find(|issue| issue.reason == Reason::ReductionDifference)
        .unwrap();
    assert_eq!(reduction.track, "HardSingle");
    assert!((reduction.start - timing.seconds(490)).abs() < 1e-9);
    assert_eq!(merged.musical_acceptance, "unverified");
}

#[test]
fn rule_marker_merge_rejects_findings_for_preserved_tracks() {
    use chart_engine::{
        pipeline,
        review_regions::{Policy, ReviewMap},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let timing = TempoMap::constant(120.0).unwrap();
    let generated = tracks("ExpertSingle", vec![note(480, 0, 100), note(500, 0, 0)]);
    let map = ReviewMap::build(
        4.0,
        Policy::default(),
        BTreeMap::new(),
        Some(BTreeSet::from(["ExpertDrums".into()])),
        BTreeSet::new(),
        vec![],
    )
    .unwrap();
    assert!(pipeline::with_generated_rule_reviews(map, &assess(&generated, &timing, 4.0)).is_err());
}

#[test]
fn export_gate_rejects_generated_errors_before_publication_and_spares_authored_tracks() {
    use chart_engine::{
        chart,
        pipeline::{self, Build, Metadata},
    };
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let audio = source.join("song.ogg");
    std::fs::write(
        &audio,
        b"opaque audio fixture; build preserves bytes without decoding",
    )
    .unwrap();
    let timing = TempoMap::constant(120.0).unwrap();
    let authored: Tracks = ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .map(|tier| {
            (
                format!("{tier}Single"),
                vec![note(0, 0, 0), note(0, 6, 240)],
            )
        })
        .collect();
    let original = chart::new_chart("Fixture", "Artist", &timing, &authored);
    std::fs::write(source.join("notes.chart"), &original).unwrap();
    let events = Transcription {
        guitar: vec![PitchNote {
            start: 0.25,
            end: 10.0,
            pitch: 60,
            confidence: 0.9,
        }],
        drums: vec![DrumHit {
            time: 0.5,
            voice: Voice::Kick,
            confidence: 0.9,
        }],
        ..Transcription::default()
    };
    let metadata = || Metadata {
        name: "Fixture".into(),
        artist: "Artist".into(),
        duration: 1.0,
        ..Metadata::default()
    };
    let rejected = root.path().join("rejected");
    let error = pipeline::build(Build {
        audio: &audio,
        output: &rejected,
        existing: None,
        timing: Some(timing.clone()),
        transcription: &events,
        meta: metadata(),
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap_err();
    assert!(error.to_string().contains("structural rules"));
    assert!(!rejected.exists());
    let output = root.path().join("enhanced");
    let receipt = pipeline::build(Build {
        audio: &audio,
        output: &output,
        existing: Some(&source),
        timing: None,
        transcription: &events,
        meta: metadata(),
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    assert_eq!(receipt["charting_rules"]["assessed_tracks"], 4);
    assert!(
        receipt["charting_rules"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["track"].as_str().unwrap().ends_with("Drums"))
    );
    let enhanced = std::fs::read_to_string(output.join("notes.chart")).unwrap();
    assert!(enhanced.starts_with(&original));
    assert_eq!(
        std::fs::read_to_string(source.join("notes.chart")).unwrap(),
        original
    );
    let map: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&std::fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    assert!(
        map.checks_performed
            .contains(&chart_engine::review_regions::Reason::SustainOverlap)
    );
}
