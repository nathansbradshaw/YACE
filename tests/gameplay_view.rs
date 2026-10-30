use chart_engine::{
    chart::Note,
    gameplay_view::{self, DrumMode, Dynamics, GemShape, Policy, Technique, ThresholdSource},
    timing::{Tempo, TempoMap},
};
use std::collections::BTreeMap;

fn note(tick: u64, lane: u8) -> Note {
    Note {
        tick,
        lane,
        sustain: 0,
    }
}
fn policy(ppq: u32) -> Policy {
    gameplay_view::yarg_chart_policy(ppq, &BTreeMap::new()).unwrap()
}

#[test]
fn piano_views_keep_keys_and_pitches_without_guitar_articulations() {
    let timing = TempoMap::constant(120.0).unwrap();
    let keys = gameplay_view::classify(
        "ExpertKeyboard",
        &[
            note(0, 0),
            note(50, 1),
            note(50, 5),
            note(50, 6),
            note(100, 7),
        ],
        &timing,
        &policy(480),
    )
    .unwrap();
    assert_eq!(keys.iter().map(|gem| gem.lane).collect::<Vec<_>>(), [0, 1]);
    assert!(keys.iter().all(|gem| gem.technique.is_none()
        && gem.natural_hopo.is_none()
        && !gem.forced_marker
        && !gem.tap_marker));
    let pitches: Vec<_> = (48..=72)
        .map(|lane| Note {
            tick: 100,
            lane,
            sustain: 480,
        })
        .collect();
    let pro =
        gameplay_view::classify("ExpertProKeys", &pitches, &timing, &Policy::unknown()).unwrap();
    assert_eq!(pro.len(), 25);
    assert_eq!(pro.first().unwrap().lane, 48);
    assert_eq!(pro.last().unwrap().lane, 72);
    assert!(
        pro.iter()
            .all(|gem| gem.attack_size == 25 && gem.technique.is_none())
    );
    assert!((pro[0].end_seconds - pro[0].seconds - 0.5).abs() < 1e-9);
}

#[test]
fn resolves_confirmed_chart_library_thresholds_and_ini_precedence() {
    assert!(gameplay_view::yarg_chart_policy(0, &BTreeMap::new()).is_err());
    assert_eq!(policy(192).hopo_threshold_ticks, Some(65));
    assert_eq!(policy(480).hopo_threshold_ticks, Some(162));
    assert_eq!(policy(96).hopo_threshold_ticks, Some(32));
    assert_eq!(
        policy(191).hopo_threshold_ticks,
        Some(63),
        "song-library and direct-parser defaults can differ at unusual PPQ"
    );
    assert!(!policy(192).chord_hopo_cancellation);
    let mut ini = BTreeMap::from([
        ("hopo_frequency".into(), "70".into()),
        ("eighthnote_hopo".into(), "true".into()),
        ("hopofreq".into(), "5".into()),
    ]);
    let resolved = gameplay_view::yarg_chart_policy(192, &ini).unwrap();
    assert_eq!(resolved.hopo_threshold_ticks, Some(70));
    assert_eq!(resolved.threshold_source, ThresholdSource::SongIniFrequency);
    ini.insert("hopo_frequency".into(), "0".into());
    assert_eq!(
        gameplay_view::yarg_chart_policy(192, &ini)
            .unwrap()
            .hopo_threshold_ticks,
        Some(97)
    );
    ini.insert("eighthnote_hopo".into(), "false".into());
    assert_eq!(
        gameplay_view::yarg_chart_policy(192, &ini)
            .unwrap()
            .hopo_threshold_ticks,
        Some(65)
    );
    ini.remove("eighthnote_hopo");
    assert_eq!(
        gameplay_view::yarg_chart_policy(192, &ini)
            .unwrap()
            .hopo_threshold_ticks,
        Some(193)
    );
    ini.insert("hopofreq".into(), "9".into());
    assert!(gameplay_view::yarg_chart_policy(192, &ini).is_err());
}

#[test]
fn distinguishes_strums_hopos_repeats_threshold_boundaries_and_forced_inversion() {
    let timing = TempoMap::new(
        192,
        -0.25,
        vec![Tempo {
            tick: 0,
            bpm: 120.0,
        }],
    )
    .unwrap();
    let notes = vec![
        note(0, 0),
        note(65, 1),
        note(130, 1),
        note(196, 2),
        note(261, 3),
        note(261, 5),
        note(326, 3),
        note(326, 5),
    ];
    let visuals = gameplay_view::classify("ExpertSingle", &notes, &timing, &policy(192)).unwrap();
    let techniques: Vec<_> = visuals.iter().map(|gem| gem.technique.unwrap()).collect();
    assert_eq!(
        techniques,
        [
            Technique::Strum,
            Technique::Hopo,
            Technique::Strum,
            Technique::Strum,
            Technique::Strum,
            Technique::Hopo
        ]
    );
    assert_eq!(visuals[4].natural_hopo, Some(true));
    assert!(visuals[4].forced_marker);
    assert_eq!(visuals[5].natural_hopo, Some(false));
    assert!(
        visuals[0].seconds < 0.0,
        "negative source offsets remain meaningful"
    );
    let first_forced = gameplay_view::classify(
        "ExpertSingle",
        &[note(0, 0), note(0, 5)],
        &timing,
        &policy(192),
    )
    .unwrap();
    assert_eq!(first_forced[0].technique, Some(Technique::Hopo));
}

#[test]
fn tap_wins_and_chords_and_opens_keep_their_actual_techniques() {
    let timing = TempoMap::constant(120.0).unwrap();
    let notes = vec![
        note(0, 0),
        note(0, 1),
        note(100, 0),
        note(200, 1),
        note(200, 2),
        note(200, 5),
        note(300, 7),
        note(300, 5),
        note(300, 6),
    ];
    let visuals =
        gameplay_view::classify("ExpertDoubleBass", &notes, &timing, &policy(480)).unwrap();
    assert!(
        visuals[..2]
            .iter()
            .all(|gem| gem.technique == Some(Technique::Strum))
    );
    assert_eq!(
        visuals[2].technique,
        Some(Technique::Hopo),
        "single after a chord may repeat one of its frets in .chart"
    );
    assert!(
        visuals[3..5]
            .iter()
            .all(|gem| gem.technique == Some(Technique::Hopo)),
        "forced chord HOPO is supported"
    );
    assert_eq!(visuals[5].shape, GemShape::Open);
    assert_eq!(visuals[5].technique, Some(Technique::Tap));
    assert!(visuals[5].forced_marker && visuals[5].tap_marker);
    let mut cancellation = policy(480);
    cancellation.chord_hopo_cancellation = true;
    assert_eq!(
        gameplay_view::classify("ExpertSingle", &notes, &timing, &cancellation).unwrap()[2]
            .technique,
        Some(Technique::Strum)
    );
}

#[test]
fn unknown_threshold_stays_unknown_instead_of_turning_forced_into_hopo() {
    let timing = TempoMap::constant(120.0).unwrap();
    let notes = vec![
        note(0, 0),
        note(100, 1),
        note(100, 5),
        note(200, 2),
        note(200, 6),
    ];
    let visuals =
        gameplay_view::classify("HardSingle", &notes, &timing, &Policy::unknown()).unwrap();
    assert_eq!(visuals[0].technique, Some(Technique::Strum));
    assert_eq!(visuals[1].natural_hopo, None);
    assert_eq!(visuals[1].technique, Some(Technique::Unknown));
    assert!(visuals[1].forced_marker);
    assert_eq!(visuals[2].technique, Some(Technique::Tap));
}

#[test]
fn drum_shapes_and_dynamics_follow_mode_and_accent_priority() {
    let timing = TempoMap::constant(120.0).unwrap();
    let notes = vec![
        note(0, 0),
        note(100, 32),
        note(200, 1),
        note(200, 34),
        note(200, 40),
        note(300, 2),
        note(300, 66),
        note(300, 41),
        note(400, 3),
        note(400, 4),
        note(400, 68),
    ];
    let visuals = gameplay_view::classify("ExpertDrums", &notes, &timing, &policy(480)).unwrap();
    assert_eq!(
        visuals.iter().map(|gem| gem.shape).collect::<Vec<_>>(),
        [
            GemShape::Kick,
            GemShape::DoubleKick,
            GemShape::Pad,
            GemShape::Cymbal,
            GemShape::Tom,
            GemShape::Cymbal
        ]
    );
    assert_eq!(visuals[2].dynamics, Dynamics::Accent);
    assert_eq!(visuals[3].dynamics, Dynamics::Ghost);
    assert!(visuals.iter().all(|gem| gem.technique.is_none()));
    let mut standard = policy(480);
    standard.drums = DrumMode::Standard;
    assert!(
        gameplay_view::classify("ExpertDrums", &notes, &timing, &standard)
            .unwrap()
            .iter()
            .all(|gem| gem.shape != GemShape::Cymbal)
    );
}

#[test]
fn classification_uses_whole_track_and_ppq_ticks_not_local_bpm() {
    let timing = TempoMap::new(
        480,
        0.5,
        vec![
            Tempo {
                tick: 0,
                bpm: 120.0,
            },
            Tempo {
                tick: 480,
                bpm: 60.0,
            },
        ],
    )
    .unwrap();
    let notes = vec![
        note(400, 0),
        Note {
            tick: 500,
            lane: 1,
            sustain: 480,
        },
    ];
    let original = notes.clone();
    let visuals = gameplay_view::classify("ExpertSingle", &notes, &timing, &policy(480)).unwrap();
    assert_eq!(visuals[1].technique, Some(Technique::Hopo));
    assert!((visuals[1].end_seconds - visuals[1].seconds - 1.0).abs() < 1e-9);
    let viewport_only =
        gameplay_view::classify("ExpertSingle", &notes[1..], &timing, &policy(480)).unwrap();
    assert_eq!(
        viewport_only[0].technique,
        Some(Technique::Strum),
        "caller must classify before viewport filtering"
    );
    assert_eq!(notes, original);
    assert!(
        gameplay_view::classify("ExpertSingle", &notes, &timing, &policy(192)).is_err(),
        "a threshold resolved for another source PPQ must not be reused"
    );
}
