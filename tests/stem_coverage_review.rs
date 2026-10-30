use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    mapping::{PitchNote, Transcription},
    models,
    review_regions::{Reason, ReviewStatus},
    stem_coverage::{
        AlignmentObservation, Candidate, CoverageReport, PitchSupport, Policy, TierCoverage,
        TimingSummary, TrackCoverage,
    },
    stem_coverage_review::{review_map_with_inputs, validate_chart_events},
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
    timing::TempoMap,
};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

fn pitch(midi: f64) -> PitchSupport {
    PitchSupport {
        midi,
        periodicity: 0.97,
        fundamental_fraction: 0.6,
        competitor_fundamental_ratio: 0.1,
    }
}
fn candidate(seconds: f64, bass: bool) -> Candidate {
    Candidate {
        seconds,
        tick: (seconds * 960.0) as u64,
        gap_start: 0.0,
        gap_end: 4.0,
        nearest_expert_attack_seconds: Some(1.0),
        flux: 1.0,
        rms: 0.1,
        envelope_ratio: 2.0,
        pitch: bass.then(|| pitch(40.0)),
        transcription_event_present: false,
        interpretation: "Uncertain isolated attack, inspect bleed and sustain coverage".into(),
        proposed_bass_event: bass.then_some(PitchNote {
            start: seconds,
            end: seconds + 0.3,
            pitch: 40,
            confidence: 0.0,
        }),
        review_required: true,
    }
}
fn report() -> CoverageReport {
    let manifest = StemManifest {
        version: 1,
        source_audio_sha256: "11".repeat(32),
        sample_rate: 44100,
        channels: 2,
        frames: 24 * 44100,
        alignment: AlignmentEvidence {
            method: "supplied_fixture".into(),
            details: "Synthetic report fixture; no musical or phase approval".into(),
        },
        stems: [StemId::Bass, StemId::Guitar, StemId::Drums]
            .into_iter()
            .enumerate()
            .map(|(i, id)| StemAsset {
                id,
                path: PathBuf::from(format!("/tmp/coverage-fixture-{}.wav", id.as_str())),
                sha256: format!("{:02x}", i + 4).repeat(32),
            })
            .collect(),
        generation: None,
    };
    let bass = TrackCoverage {
        stem: StemId::Bass,
        expert_track: "ExpertDoubleBass".into(),
        expert_attack_positions: 3,
        transcription_scope: "Bound fixture raw events".into(),
        flux_peaks: 4,
        strong_peaks_in_long_gaps: 1,
        rejected_bass_ambiguity: 0,
        candidates: vec![candidate(2.0, true)],
        tiers: vec![TierCoverage {
            track: "EasyDoubleBass".into(),
            attack_positions: 0,
            expert_positions_not_in_tier: 3,
            tier_positions_not_in_expert: 0,
            interpretation: "Arrangement summary only".into(),
        }],
        alignment: vec![
            AlignmentObservation {
                tick: 5760,
                seconds: 6.0,
                nearest_flux_seconds: Some(6.075),
                audio_minus_chart_seconds: Some(0.075),
                supported_attack: true,
                raw_transcription_seconds: Some(6.0),
                raw_transcription_pitch: Some(40),
                raw_minus_chart_seconds: Some(0.0),
                periodic_pitch: Some(pitch(42.1)),
                periodic_minus_raw_semitones: Some(2.1),
                pitch_window: None,
                proposed_attack_tick: Some(5832),
                review_required: true,
            },
            AlignmentObservation {
                tick: 7680,
                seconds: 8.0,
                nearest_flux_seconds: Some(8.05),
                audio_minus_chart_seconds: Some(0.05),
                supported_attack: true,
                raw_transcription_seconds: Some(8.0),
                raw_transcription_pitch: Some(40),
                raw_minus_chart_seconds: Some(0.0),
                periodic_pitch: Some(pitch(41.0)),
                periodic_minus_raw_semitones: Some(1.0),
                pitch_window: None,
                proposed_attack_tick: None,
                review_required: true,
            },
            AlignmentObservation {
                tick: 9600,
                seconds: 10.0,
                nearest_flux_seconds: Some(10.12),
                audio_minus_chart_seconds: Some(0.12),
                supported_attack: false,
                raw_transcription_seconds: None,
                raw_transcription_pitch: None,
                raw_minus_chart_seconds: None,
                periodic_pitch: None,
                periodic_minus_raw_semitones: None,
                pitch_window: None,
                proposed_attack_tick: None,
                review_required: true,
            },
        ],
        timing: TimingSummary::default(),
    };
    let guitar = TrackCoverage {
        stem: StemId::Guitar,
        expert_track: "ExpertSingle".into(),
        expert_attack_positions: 0,
        transcription_scope: "No raw guitar events".into(),
        flux_peaks: 1,
        strong_peaks_in_long_gaps: 1,
        rejected_bass_ambiguity: 0,
        candidates: vec![candidate(3.0, false)],
        tiers: vec![],
        alignment: vec![],
        timing: TimingSummary::default(),
    };
    CoverageReport {
        version: 1,
        source_audio_sha256: manifest.source_audio_sha256.clone(),
        chart_sha256: "22".repeat(32),
        transcription_sha256: Some("33".repeat(32)),
        manifest,
        policy: Policy::default(),
        tracks: vec![bass, guitar],
        limits: vec!["Original detector limitation retained exactly".into()],
    }
}

fn merge_base(
    derived: &chart_engine::review_regions::ReviewMap,
) -> chart_engine::review_regions::ReviewMap {
    use chart_engine::review_regions::{Issue, Policy, ReviewMap};
    use std::collections::BTreeSet;
    let duplicate = derived
        .regions
        .iter()
        .flat_map(|region| &region.issues)
        .next()
        .unwrap()
        .clone();
    ReviewMap::build(
        derived.duration_seconds,
        Policy {
            window_seconds: 6.,
            ..Policy::default()
        },
        BTreeMap::from([
            ("chart".into(), derived.input_sha256["chart"].clone()),
            (
                "original_audio".into(),
                derived.input_sha256["original_audio"].clone(),
            ),
            ("generation".into(), "aa".repeat(32)),
        ]),
        Some(BTreeSet::from([
            "ExpertDoubleBass".into(),
            "EasyDoubleBass".into(),
        ])),
        BTreeSet::from([Reason::FastRepeat]),
        vec![
            duplicate,
            Issue::at(0.5, "ExpertDrums", Reason::FastRepeat, &[]),
        ],
    )
    .unwrap()
}

#[test]
fn merge_preserves_genuine_generation_scope_prior_flags_and_base_policy_without_approval() {
    use chart_engine::stem_coverage_review::merge_into_base;
    let derived = report().review_map(24.).unwrap();
    let mut base = merge_base(&derived);
    base.coverage_limits
        .push("Earlier diagnostic limitation".into());
    base.musical_acceptance = "untrusted prior claim must not carry".into();
    let merged = merge_into_base(&derived, &base, &"bb".repeat(32)).unwrap();
    assert_eq!(merged.generated_tracks, base.generated_tracks);
    assert_eq!(merged.policy, base.policy);
    assert_eq!(merged.musical_acceptance, "unverified");
    assert!(
        merged
            .regions
            .iter()
            .all(|region| region.status == ReviewStatus::Pending)
    );
    assert!(merged.checks_performed.contains(&Reason::FastRepeat));
    assert!(merged.checks_performed.contains(&Reason::StemAttackTiming));
    let original_count: usize = derived
        .regions
        .iter()
        .map(|region| region.issues.len())
        .sum();
    assert_eq!(
        merged
            .regions
            .iter()
            .map(|region| region.issues.len())
            .sum::<usize>(),
        original_count + 1
    );
    assert!(
        merged
            .coverage_limits
            .contains(&"Earlier diagnostic limitation".into())
    );
    assert_eq!(
        merged.input_sha256["base_review_markers_file"],
        "bb".repeat(32)
    );
}

#[test]
fn merge_rejects_wrong_recording_duration_and_conflicting_evidence_even_for_same_chart() {
    use chart_engine::stem_coverage_review::merge_into_base;
    let derived = report().review_map(24.).unwrap();
    let base = merge_base(&derived);
    let mut wrong = base.clone();
    wrong
        .input_sha256
        .insert("original_audio".into(), "cc".repeat(32));
    assert!(merge_into_base(&derived, &wrong, &"bb".repeat(32)).is_err());
    let mut wrong = base.clone();
    wrong.duration_seconds += 0.001;
    assert!(merge_into_base(&derived, &wrong, &"bb".repeat(32)).is_err());
    let mut wrong = base.clone();
    wrong.input_sha256.insert("chart".into(), "cc".repeat(32));
    assert!(merge_into_base(&derived, &wrong, &"bb".repeat(32)).is_err());
    let mut wrong = base;
    wrong
        .input_sha256
        .insert("transcription".into(), "cc".repeat(32));
    assert!(merge_into_base(&derived, &wrong, &"bb".repeat(32)).is_err());
}

#[test]
fn serialized_base_identity_changes_merged_region_ids_deterministically() {
    use chart_engine::stem_coverage_review::merge_into_base;
    let derived = report().review_map(24.).unwrap();
    let base = merge_base(&derived);
    let first = merge_into_base(&derived, &base, &"bb".repeat(32)).unwrap();
    let same = merge_into_base(&derived, &base, &"bb".repeat(32)).unwrap();
    let different = merge_into_base(&derived, &base, &"cc".repeat(32)).unwrap();
    assert_eq!(first.regions, same.regions);
    assert_ne!(first.regions[0].id, different.regions[0].id);
}

#[test]
fn supported_observations_become_pending_expert_flags_without_edits_or_tier_additions() {
    let report = report();
    let before = serde_json::to_vec(&report).unwrap();
    let map = report.review_map(24.0).unwrap();
    assert_eq!(serde_json::to_vec(&report).unwrap(), before);
    assert_eq!(map.musical_acceptance, "unverified");
    assert!(map.generated_tracks.is_none());
    assert!(
        map.regions
            .iter()
            .all(|region| region.status == ReviewStatus::Pending)
    );
    let issues: Vec<_> = map.regions.iter().flat_map(|r| &r.issues).collect();
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.reason == Reason::UnchartedStemAttack)
            .count(),
        2
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.reason == Reason::StemAttackTiming)
            .count(),
        1
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.reason == Reason::PitchDisagreement)
            .count(),
        1
    );
    assert!(issues.iter().all(|issue| issue.track.starts_with("Expert")));
    assert!(
        issues
            .iter()
            .all(|issue| issue.start >= 0.0 && issue.end <= 24.0)
    );
    let timing = issues
        .iter()
        .find(|i| i.reason == Reason::StemAttackTiming)
        .unwrap();
    assert_eq!(timing.evidence["audio_minus_chart_seconds"], 0.075);
    assert_eq!(timing.evidence["proposed_tick_unapproved"], 5832.0);
    assert!(map.coverage_limits.contains(&report.limits[0]));
    assert!(
        map.coverage_limits
            .iter()
            .any(|l| l.contains("not physical onset truth"))
    );
    assert!(map.unflagged_sections.contains("unverified"));
    assert!(map.checks_performed.contains(&Reason::UnchartedStemAttack));
    assert!(map.checks_performed.contains(&Reason::StemAttackTiming));
    for reason in [Reason::UnchartedStemAttack, Reason::StemAttackTiming] {
        assert!(Reason::ALL.contains(&reason));
        assert!(reason.description().contains("before"));
        let decoded: Reason =
            serde_json::from_str(&serde_json::to_string(&reason).unwrap()).unwrap();
        assert_eq!(decoded, reason);
        assert!(!reason.label().is_empty());
    }
}

#[test]
fn all_inputs_bind_region_identity_and_file_hashes_cannot_override_analyzer_sources() {
    let report = report();
    let a = report.review_map(24.0).unwrap();
    let b = report.review_map(24.0).unwrap();
    let ids = |map: &chart_engine::review_regions::ReviewMap| {
        map.regions.iter().map(|r| r.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(&a), ids(&b));
    for key in [
        "chart",
        "original_audio",
        "transcription",
        "stem_manifest_semantic",
        "stem_coverage_semantic",
        "stem:bass",
        "stem:guitar",
        "stem:drums",
    ] {
        assert!(a.input_sha256.contains_key(key), "{key}");
    }
    let files = BTreeMap::from([
        ("stem_manifest_file".into(), "aa".repeat(32)),
        ("stem_coverage_report_file".into(), "bb".repeat(32)),
    ]);
    let withfiles = review_map_with_inputs(&report, 24.0, files).unwrap();
    assert_ne!(ids(&a), ids(&withfiles));
    assert_eq!(
        withfiles.input_sha256["stem_manifest_file"],
        "aa".repeat(32)
    );
    let mut changed = report.clone();
    changed.chart_sha256 = "77".repeat(32);
    assert_ne!(ids(&a), ids(&changed.review_map(24.0).unwrap()));
    changed = report.clone();
    changed.transcription_sha256 = Some("88".repeat(32));
    assert_ne!(ids(&a), ids(&changed.review_map(24.0).unwrap()));
    changed = report.clone();
    changed.manifest.stems[0].sha256 = "99".repeat(32);
    assert_ne!(ids(&a), ids(&changed.review_map(24.0).unwrap()));
    for key in [
        "chart",
        "original_audio",
        "transcription",
        "stem:bass",
        "separation_model",
    ] {
        assert!(
            review_map_with_inputs(
                &report,
                24.0,
                BTreeMap::from([(key.into(), "ab".repeat(32))])
            )
            .is_err()
        );
    }
}

#[test]
fn weak_or_unreviewable_evidence_remains_unflagged_and_never_certifies_completeness() {
    let mut report = report();
    report.tracks[0].candidates[0]
        .pitch
        .as_mut()
        .unwrap()
        .periodicity = 0.5;
    report.tracks[1].candidates[0].review_required = false;
    for o in &mut report.tracks[0].alignment {
        o.review_required = false;
    }
    let map = report.review_map(24.0).unwrap();
    assert!(map.regions.is_empty());
    assert_eq!(map.musical_acceptance, "unverified");
    assert!(map.checks_performed.contains(&Reason::StemAttackTiming));
    assert!(map.unflagged_sections.contains("unverified"));
}

#[test]
fn mismatched_hashes_durations_and_nonfinite_or_inconsistent_evidence_are_rejected() {
    let valid = report();
    for duration in [0.0, -1.0, f64::NAN, f64::INFINITY, 23.99, 25.0] {
        assert!(valid.review_map(duration).is_err());
    }
    let mut bad = valid.clone();
    bad.source_audio_sha256 = "ff".repeat(32);
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.chart_sha256 = "unknown".into();
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.transcription_sha256 = None;
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].candidates[0].rms = f64::NAN;
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].candidates[0].gap_end = 25.0;
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].candidates[0]
        .proposed_bass_event
        .as_mut()
        .unwrap()
        .confidence = 0.9;
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].alignment[0].audio_minus_chart_seconds = Some(-0.075);
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].alignment[0].periodic_minus_raw_semitones = Some(-2.1);
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks[0].expert_track = "EasyDoubleBass".into();
    assert!(bad.review_map(24.0).is_err());
    bad = valid.clone();
    bad.tracks.push(bad.tracks[0].clone());
    assert!(bad.review_map(24.0).is_err());
}

#[test]
fn bounds_validation_preserves_negative_chart_offsets_but_rejects_events_past_audio() {
    let mut timing = TempoMap::constant(120.0).unwrap();
    timing.offset = -0.1;
    let tracks = Tracks::from([(
        "ExpertSingle".into(),
        vec![
            Note {
                tick: 0,
                lane: 0,
                sustain: 0,
            },
            Note {
                tick: 100_000,
                lane: 99,
                sustain: 17,
            },
        ],
    )]);
    let chart = Chart::parse(chart::new_chart("Bounds", "Test", &timing, &tracks)).unwrap();
    validate_chart_events(&chart, None, 1.0).unwrap();
    let events = Transcription {
        bass: vec![PitchNote {
            start: 0.8,
            end: 1.2,
            pitch: 40,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    assert!(validate_chart_events(&chart, Some(&events), 1.0).is_err());
}

#[test]
fn audit_cli_publishes_both_bound_files_and_refuses_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let audio = root.path().join("song.wav");
    let stem = root.path().join("bass.wav");
    models::write_audio(&audio, &vec![0.0; 44100], &vec![0.0; 44100]).unwrap();
    models::write_audio(&stem, &vec![0.0; 44100], &vec![0.0; 44100]).unwrap();
    let manifest = StemManifest::build(
        &audio,
        BTreeMap::from([(StemId::Bass, stem)]),
        AlignmentEvidence {
            method: "synthetic_silence_fixture".into(),
            details: "Same source clock; no phase or musical quality claim".into(),
        },
    )
    .unwrap();
    let path = root.path().join("manifest.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let chart = root.path().join("notes.chart");
    fs::write(
        &chart,
        chart::new_chart(
            "Audit",
            "Test",
            &TempoMap::constant(120.0).unwrap(),
            &Tracks::from([("ExpertDoubleBass".into(), vec![])]),
        ),
    )
    .unwrap();
    let output = root.path().join("audit");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_audit-stems"))
            .arg("--audio")
            .arg(&audio)
            .arg("--chart")
            .arg(&chart)
            .arg("--manifest")
            .arg(&path)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(info["status"], "source_bound_diagnostics");
    assert_eq!(info["musical_acceptance"], "unverified");
    let coverage: CoverageReport =
        serde_json::from_slice(&fs::read(output.join("coverage.json")).unwrap()).unwrap();
    let flags: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("review-markers.json")).unwrap()).unwrap();
    assert_eq!(
        flags.input_sha256["original_audio"],
        coverage.source_audio_sha256
    );
    assert_eq!(
        flags.input_sha256["chart"],
        models::hash_file(&chart).unwrap()
    );
    assert_eq!(
        flags.input_sha256["stem_manifest_file"],
        models::hash_file(&path).unwrap()
    );
    assert_eq!(
        flags.input_sha256["stem_coverage_report_file"],
        models::hash_file(&output.join("coverage.json")).unwrap()
    );
    let old = fs::read(output.join("review-markers.json")).unwrap();
    assert!(!run().status.success());
    assert_eq!(fs::read(output.join("review-markers.json")).unwrap(), old);
}
