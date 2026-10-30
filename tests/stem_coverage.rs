use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    mapping::{PitchNote, Transcription},
    stem_coverage::{Policy, analyze_stem, audit_files},
    stem_review::StemId,
    timing::TempoMap,
};
use std::{fs, path::PathBuf};

fn song(expert: &[f64], easy: &[f64]) -> Chart {
    let timing = TempoMap::constant(120.).unwrap();
    let mut tracks = Tracks::new();
    for (track, attacks) in [("ExpertDoubleBass", expert), ("EasyDoubleBass", easy)] {
        tracks.insert(
            track.into(),
            attacks
                .iter()
                .map(|time| Note {
                    tick: timing.tick(*time).unwrap(),
                    lane: 0,
                    sustain: 0,
                })
                .collect(),
        );
    }
    Chart::parse(chart::new_chart(
        "Original tone fixture",
        "Test",
        &timing,
        &tracks,
    ))
    .unwrap()
}
fn signal(rate: usize, starts: &[f64], frequency: f64) -> Vec<f32> {
    (0..rate * 4)
        .map(|i| {
            let time = i as f64 / rate as f64;
            starts
                .iter()
                .map(|start| {
                    let age = time - start;
                    if !(0.0..0.6).contains(&age) {
                        0.
                    } else {
                        let envelope = (age / 0.008).min(1.) * (1. - age / 0.6).max(0.);
                        (std::f64::consts::TAU * frequency * age).sin() * 0.3 * envelope
                    }
                })
                .sum::<f64>() as f32
        })
        .collect()
}
#[test]
fn a_later_stable_pitch_cannot_certify_an_earlier_octave_transition() {
    let timing = TempoMap::constant(120.).unwrap();
    let events = Transcription {
        bass: vec![
            PitchNote {
                start: 0.2,
                end: 0.4,
                pitch: 52,
                confidence: 0.8,
            },
            PitchNote {
                start: 0.24,
                end: 0.9,
                pitch: 40,
                confidence: 0.9,
            },
        ],
        ..Default::default()
    };
    let tracks = chart_engine::mapping::tracks(&events, &timing, false).unwrap();
    let chart = Chart::parse(chart::new_chart("Octave transition", "", &timing, &tracks)).unwrap();
    let audio = |rate: usize| {
        (0..rate * 2)
            .map(|i| {
                let time = i as f64 / rate as f64;
                let frequency = if (0.2..0.22).contains(&time) {
                    164.81378
                } else if (0.22..0.9).contains(&time) {
                    82.40689
                } else {
                    return 0.;
                };
                // A soft real high note followed by a louder octave drop.
                // The later centered windows can look stable despite crossing
                // the short onset's ownership boundary.
                let amplitude = if time < 0.22 { 0.03 } else { 0.3 };
                (amplitude * (std::f64::consts::TAU * frequency * time).sin()) as f32
            })
            .collect::<Vec<_>>()
    };
    let report = analyze_stem(
        &chart,
        StemId::Bass,
        &audio(22050),
        &audio(11025),
        &[],
        Some(&events),
        &Default::default(),
    )
    .unwrap();
    let observation = &report.alignment[0];
    assert!(
        observation
            .periodic_minus_raw_semitones
            .is_some_and(|difference| difference.abs() > 11.),
        "{report:#?}"
    );
    let window = observation.pitch_window.as_ref().unwrap();
    assert!(!window.owned);
    assert_eq!(window.competing_candidate_start, Some(0.24));
    assert!(observation.review_required);
    assert!(observation.proposed_attack_tick.is_none());
    assert_eq!(report.timing.inconclusive_pitch_windows, 1);
    assert_eq!(report.timing.owned_pitch_disagreements, 0);
}

#[test]
fn sustained_bass_attacks_in_empty_expert_gaps_become_unapproved_event_candidates() {
    let chart = song(&[], &[]);
    let spectral = signal(22050, &[1.], 82.40689);
    let pitch = signal(11025, &[1.], 82.40689);
    let result = analyze_stem(
        &chart,
        StemId::Bass,
        &spectral,
        &pitch,
        &[],
        Some(&Transcription::default()),
        &Policy::default(),
    )
    .unwrap();
    assert!(!result.candidates.is_empty(), "{result:?}");
    let candidate = result
        .candidates
        .iter()
        .find(|c| (c.seconds - 1.).abs() < 0.08)
        .unwrap();
    assert_eq!(candidate.proposed_bass_event.as_ref().unwrap().pitch, 40);
    assert_eq!(
        candidate.proposed_bass_event.as_ref().unwrap().confidence,
        0.
    );
    assert!(candidate.review_required);
    assert_eq!(chart.timing.tick(candidate.seconds), Some(candidate.tick));
}
#[test]
fn matching_louder_competing_fundamental_vetoes_bass_patch() {
    let chart = song(&[], &[]);
    let spectral = signal(22050, &[1.], 82.40689);
    let pitch = signal(11025, &[1.], 82.40689);
    let competitor: Vec<_> = pitch.iter().map(|v| v * 2.).collect();
    let result = analyze_stem(
        &chart,
        StemId::Bass,
        &spectral,
        &pitch,
        &[&competitor],
        None,
        &Policy::default(),
    )
    .unwrap();
    assert!(result.strong_peaks_in_long_gaps > 0);
    assert!(result.rejected_bass_ambiguity > 0);
    assert!(result.candidates.is_empty());
}
#[test]
fn intentional_lower_tier_omissions_do_not_create_audio_additions() {
    let chart = song(&[1., 2.], &[1.]);
    let spectral = signal(22050, &[1., 2.], 82.40689);
    let pitch = signal(11025, &[1., 2.], 82.40689);
    let result = analyze_stem(
        &chart,
        StemId::Bass,
        &spectral,
        &pitch,
        &[],
        None,
        &Policy::default(),
    )
    .unwrap();
    assert!(result.candidates.is_empty());
    let easy = &result.tiers[0];
    assert_eq!(easy.expert_positions_not_in_tier, 1);
    assert_eq!(easy.tier_positions_not_in_expert, 0);
}
#[test]
fn local_audio_timing_and_raw_mapping_residuals_remain_distinct() {
    // The known source attack is 1.0s; deliberately map it 100ms early.
    // Centered spectral windows can see its leading edge before that sample.
    let chart = song(&[0.90], &[]);
    let spectral = signal(22050, &[1.], 82.40689);
    let pitch = signal(11025, &[1.], 82.40689);
    let events = Transcription {
        bass: vec![PitchNote {
            start: 0.90,
            end: 1.5,
            pitch: 40,
            confidence: 0.8,
        }],
        ..Default::default()
    };
    let result = analyze_stem(
        &chart,
        StemId::Bass,
        &spectral,
        &pitch,
        &[],
        Some(&events),
        &Policy::default(),
    )
    .unwrap();
    let observation = &result.alignment[0];
    assert!(observation.supported_attack, "{observation:?}");
    assert!((observation.nearest_flux_seconds.unwrap() - 1.).abs() < 0.025);
    assert!(
        observation.audio_minus_chart_seconds.unwrap() > 0.05,
        "{observation:?}"
    );
    assert!(observation.raw_minus_chart_seconds.unwrap().abs() < 0.001);
    assert!(observation.periodic_minus_raw_semitones.unwrap().abs() < 0.2);
    assert!(observation.proposed_attack_tick.is_some());
    assert!(result.timing.supported_residuals_over_50ms > 0);
}
#[test]
fn existing_transcription_event_is_mapping_review_not_duplicate_pitch_append() {
    let chart = song(&[], &[]);
    let spectral = signal(22050, &[1.], 82.40689);
    let pitch = signal(11025, &[1.], 82.40689);
    let events = Transcription {
        bass: vec![PitchNote {
            start: 1.,
            end: 1.5,
            pitch: 40,
            confidence: 0.8,
        }],
        ..Default::default()
    };
    let result = analyze_stem(
        &chart,
        StemId::Bass,
        &spectral,
        &pitch,
        &[],
        Some(&events),
        &Policy::default(),
    )
    .unwrap();
    assert!(!result.candidates.is_empty());
    assert!(
        result
            .candidates
            .iter()
            .all(|c| c.transcription_event_present && c.proposed_bass_event.is_none())
    );
}
#[test]
fn silence_invalid_policy_nonfinite_and_inconsistent_clocks_are_explicit() {
    let chart = song(&[], &[]);
    let spectral = vec![0.; 22050];
    let pitch = vec![0.; 11025];
    assert!(
        analyze_stem(
            &chart,
            StemId::Bass,
            &spectral,
            &pitch,
            &[],
            None,
            &Policy::default()
        )
        .unwrap()
        .candidates
        .is_empty()
    );
    let bad = Policy {
        long_gap_seconds: f64::NAN,
        ..Policy::default()
    };
    assert!(analyze_stem(&chart, StemId::Bass, &spectral, &pitch, &[], None, &bad).is_err());
    let mut nonfinite = pitch.clone();
    nonfinite[100] = f32::NAN;
    assert!(
        analyze_stem(
            &chart,
            StemId::Bass,
            &spectral,
            &nonfinite,
            &[],
            None,
            &Policy::default()
        )
        .is_err()
    );
    assert!(
        analyze_stem(
            &chart,
            StemId::Bass,
            &spectral,
            &pitch[..100],
            &[],
            None,
            &Policy::default()
        )
        .is_err()
    );
    assert!(
        analyze_stem(
            &chart,
            StemId::Bass,
            &spectral,
            &pitch,
            &[&pitch[..100]],
            None,
            &Policy::default()
        )
        .is_err()
    );
}
#[test]
fn guitar_and_drum_attack_markers_never_invent_pitch_or_lane_patches() {
    let chart = song(&[], &[]);
    let spectral = signal(22050, &[1.], 110.);
    let pitch = signal(11025, &[1.], 110.);
    for stem in [StemId::Guitar, StemId::Drums] {
        let result = analyze_stem(
            &chart,
            stem,
            &spectral,
            &pitch,
            &[],
            None,
            &Policy::default(),
        )
        .unwrap();
        assert!(!result.candidates.is_empty());
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.proposed_bass_event.is_none() && c.pitch.is_none())
        );
        assert!(result.alignment.is_empty());
    }
}

#[test]
fn file_audit_is_source_bound_and_preserves_the_chart_and_recording() {
    use chart_engine::{
        models,
        stem_review::{AlignmentEvidence, StemManifest},
    };
    use std::collections::BTreeMap;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.wav");
    let stem = temp.path().join("bass.wav");
    let pcm = signal(44100, &[1.], 82.40689);
    models::write_audio(&source, &pcm, &pcm).unwrap();
    models::write_audio(&stem, &pcm, &pcm).unwrap();
    let manifest=StemManifest::build(&source,BTreeMap::from([(StemId::Bass,stem.clone())]),AlignmentEvidence{method:"synthetic_additive_fixture".into(),details:"Original synthetic signal copied to separate bass fixture, with identical sample coordinates.".into()}).unwrap();
    let manifest_path = temp.path().join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let chart = temp.path().join("notes.chart");
    let original_chart = song(&[0.94], &[]).raw;
    fs::write(&chart, &original_chart).unwrap();
    let source_hash = models::hash_file(&source).unwrap();
    let report = audit_files(&source, &chart, &manifest_path, None, &Policy::default()).unwrap();
    assert_eq!(report.source_audio_sha256, source_hash);
    assert_eq!(report.tracks.len(), 1);
    assert_eq!(fs::read_to_string(&chart).unwrap(), original_chart);
    assert_eq!(models::hash_file(&source).unwrap(), source_hash);
    fs::write(&stem, b"changed").unwrap();
    assert!(audit_files(&source, &chart, &manifest_path, None, &Policy::default()).is_err());
    assert_eq!(fs::read_to_string(&chart).unwrap(), original_chart);
}

/// Local diagnostic runner. Input jobs are explicit files; never edits charts,
/// projects or existing reports. Not part of ordinary portable test execution.
#[test]
#[ignore = "requires explicit local source/stem jobs in CHART_ENGINE_COVERAGE_JOBS"]
fn audit_actual_cached_sources() {
    #[derive(serde::Deserialize)]
    struct Job {
        source: PathBuf,
        chart: PathBuf,
        manifest: PathBuf,
        transcription: Option<PathBuf>,
        output: PathBuf,
    }
    let path = std::env::var_os("CHART_ENGINE_COVERAGE_JOBS").expect("explicit jobs path required");
    let jobs: Vec<Job> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(!jobs.is_empty());
    for job in jobs {
        assert!(!job.output.exists(), "refusing overwrite");
        let report = audit_files(
            &job.source,
            &job.chart,
            &job.manifest,
            job.transcription.as_deref(),
            &Policy::default(),
        )
        .unwrap();
        let mut file = tempfile::NamedTempFile::new_in(job.output.parent().unwrap()).unwrap();
        serde_json::to_writer_pretty(&mut file, &report).unwrap();
        file.as_file().sync_all().unwrap();
        file.persist_noclobber(&job.output).unwrap();
        println!("{}", job.output.display());
    }
}
