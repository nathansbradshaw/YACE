use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    difficulty_links,
    mapping::{PitchNote, Transcription},
    pipeline::{self, Build, Metadata},
    timing::TempoMap,
};
use std::{collections::BTreeSet, fs};

fn n(tick: u64, lane: u8) -> Note {
    Note {
        tick,
        lane,
        sustain: 0,
    }
}

#[test]
fn raw_generation_uses_effective_expert_and_never_rewrites_authored_lower_or_inferred_expert() {
    let timing = TempoMap::constant(120.0).unwrap();
    let generated = Tracks::from([
        ("ExpertSingle".into(), vec![n(100, 0), n(300, 1), n(700, 2)]),
        ("HardSingle".into(), vec![n(999, 4)]),
        ("MediumSingle".into(), vec![n(999, 4)]),
        ("EasySingle".into(), vec![n(999, 4)]),
    ]);
    let authored = Tracks::from([
        (
            "ExpertSingle".into(),
            vec![
                n(480, 2),
                n(480, 4),
                n(960, 1),
                n(960, 6),
                n(1440, 3),
                Note {
                    tick: 2000,
                    lane: 99,
                    sustain: 17,
                },
            ],
        ),
        ("EasySingle".into(), vec![n(777, 4)]),
    ]);
    let (reduced, lineage) =
        difficulty_links::reduce_generated(&generated, &authored, &timing, 5.0).unwrap();
    assert_eq!(reduced["ExpertSingle"], generated["ExpertSingle"]);
    assert_eq!(
        reduced["EasySingle"], generated["EasySingle"],
        "authored lower will be preserved by additive export, not regenerated"
    );
    assert!(!lineage.contains_key("EasySingle"));
    let expert_ticks: BTreeSet<_> = authored["ExpertSingle"]
        .iter()
        .filter(|n| n.lane <= 4 || n.lane == 7)
        .map(|n| n.tick)
        .collect();
    for track in ["HardSingle", "MediumSingle"] {
        assert!(
            reduced[track]
                .iter()
                .all(|n| expert_ticks.contains(&n.tick))
        );
        assert!(reduced[track].iter().all(|n| n.lane != 99));
        assert_eq!(lineage[track].expert_track, "ExpertSingle");
        assert_eq!(lineage[track].master_origin, "authored_chart");
        assert_eq!(lineage[track].rule_id, difficulty_links::RULE_ID);
    }
    let (fresh, lineage) =
        difficulty_links::reduce_generated(&generated, &Tracks::new(), &timing, 5.0).unwrap();
    assert_eq!(fresh["ExpertSingle"], generated["ExpertSingle"]);
    assert!(
        fresh["EasySingle"]
            .iter()
            .all(|n| generated["ExpertSingle"].iter().any(|e| e.tick == n.tick))
    );
    assert_eq!(lineage["EasySingle"].master_origin, "generated_expert");
}

#[test]
fn additive_build_reduces_missing_rows_from_authored_master_and_preserves_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    let timing = TempoMap::constant(120.0).unwrap();
    let authored = Tracks::from([
        (
            "ExpertSingle".into(),
            vec![
                n(480, 4),
                n(960, 2),
                n(1440, 1),
                Note {
                    tick: 2000,
                    lane: 99,
                    sustain: 17,
                },
            ],
        ),
        ("EasySingle".into(), vec![n(777, 4)]),
    ]);
    let original = chart::new_chart("Master fixture", "Test", &timing, &authored);
    fs::write(source.join("notes.chart"), &original).unwrap();
    fs::write(
        source.join("song.ini"),
        "[song]\nname = Master fixture\nartist = Test\ncustom = keep\n",
    )
    .unwrap();
    let audio = source.join("song.ogg");
    let previous_receipt = b"{\"status\":\"prior generation retained byte for byte\"}";
    fs::write(source.join("chart-engine-report.json"), previous_receipt).unwrap();
    fs::write(
        &audio,
        b"opaque unit audio; no decoder or inference in build",
    )
    .unwrap();
    let transcription = Transcription {
        guitar: vec![
            PitchNote {
                start: 0.1,
                end: 0.2,
                pitch: 45,
                confidence: 0.4,
            },
            PitchNote {
                start: 0.3,
                end: 0.4,
                pitch: 60,
                confidence: 0.4,
            },
            PitchNote {
                start: 0.7,
                end: 0.8,
                pitch: 80,
                confidence: 0.4,
            },
        ],
        ..Default::default()
    };
    let output = root.path().join("output");
    let receipt = pipeline::build(Build {
        audio: &audio,
        output: &output,
        existing: Some(&source),
        timing: None,
        transcription: &transcription,
        meta: Metadata {
            name: "Master fixture".into(),
            artist: "Test".into(),
            duration: 5.0,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    let text = fs::read_to_string(output.join("notes.chart")).unwrap();
    let archives: Vec<_> = fs::read_dir(output.join("chart-engine-history"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(archives.len(), 1);
    assert_eq!(
        fs::read(archives[0].join("chart-engine-report.json")).unwrap(),
        previous_receipt
    );
    assert_eq!(
        fs::read(source.join("chart-engine-report.json")).unwrap(),
        previous_receipt
    );
    assert!(archives[0].join("archive-sha256.json").is_file());
    assert!(archives[0].join("archive-origin.json").is_file());
    assert!(text.starts_with(&original));
    assert_eq!(
        fs::read_to_string(source.join("notes.chart")).unwrap(),
        original
    );
    let chart = Chart::parse(text).unwrap();
    assert_eq!(
        chart.notes("ExpertSingle").unwrap(),
        authored["ExpertSingle"]
    );
    assert_eq!(chart.notes("EasySingle").unwrap(), authored["EasySingle"]);
    for track in ["HardSingle", "MediumSingle"] {
        let notes = chart.notes(track).unwrap();
        assert!(!notes.is_empty());
        assert!(notes.iter().all(|n| [480, 960, 1440].contains(&n.tick)));
        assert_eq!(
            receipt["difficulty_reduction"][track]["master_origin"],
            "authored_chart"
        );
    }
    assert!(receipt["difficulty_reduction"]["EasySingle"].is_null());
    assert!(receipt["difficulty_reduction"]["ExpertSingle"].is_null());
    let markers: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    assert!(
        !markers
            .regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == chart_engine::review_regions::Reason::LowCandidateScore)
    );
    assert_eq!(markers.musical_acceptance, "unverified");
}

#[test]
fn standard_drums_retain_yellow_without_inventing_pro_markers() {
    let timing = TempoMap::constant(120.0).unwrap();
    let source = Tracks::from([
        ("ExpertDrums".into(), vec![n(480, 2), n(960, 2), n(1440, 2)]),
        ("HardDrums".into(), vec![]),
        ("MediumDrums".into(), vec![]),
        ("EasyDrums".into(), vec![]),
    ]);
    let (result, _) =
        difficulty_links::reduce_generated(&source, &Tracks::new(), &timing, 5.0).unwrap();
    assert_eq!(result["ExpertDrums"], source["ExpertDrums"]);
    assert!(!result["EasyDrums"].is_empty());
    assert!(result["EasyDrums"].iter().all(|n| n.lane == 2));
    assert!(
        result
            .values()
            .flatten()
            .all(|n| !(66..=68).contains(&n.lane))
    );
}
