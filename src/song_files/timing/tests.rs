use super::*;
use crate::{
    chart::{Note, Tracks},
    editor::{Action, Actor, ActorKind, Request, Store},
    mapping::{DrumHit, Transcription, Voice},
    pipeline::{self, Build, Metadata},
    timing::TempoMap,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};

const CHART: &str = "\u{feff}[Song]\r\n{\r\n Resolution = 480\r\n Offset = 0.5\r\n Custom = retain\r\n}\r\n[SyncTrack]\r\n{\r\n0 = B 120000\r\n960 = B 90000\r\n}\r\n[Events]\r\n{\r\n0 = E \"section Intro\"\r\n}\r\n[ExpertSingle]\r\n{\r\n480 = N 0 120\r\n}\r\n";

fn song(root: &Path, delay: &str) -> std::path::PathBuf {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    fs::write(song.join("song.ini"), format!("[song]\r\nname = Timing fixture\r\nartist = Test\r\ndelay = {delay}\r\ncustom = keep\r\n[other]\r\ndelay = 9\r\n")).unwrap();
    let samples = vec![0.; 44100 * 3];
    crate::models::write_audio(&song.join("song.wav"), &samples, &samples).unwrap();
    song
}

#[test]
fn legacy_precedence_is_not_additive_and_invalid_values_are_explicit_errors() {
    for (ini, expected) in [
        ("delay = 250\ndelay_seconds = 2", Some(0.25)),
        ("delay = -125\ndelay_seconds = 2", Some(-0.125)),
        ("delay = 0\ndelay_seconds = -0.125", Some(-0.125)),
        ("delay = 0", None),
        ("delay_seconds = 0", None),
    ] {
        assert_eq!(legacy_delay(&format!("[song]\n{ini}\n")).unwrap(), expected);
    }
    for value in ["NaN", "inf", "-inf", "1.5", "2147483648", "not a number"] {
        assert!(legacy_delay(&format!("[song]\ndelay = {value}\n")).is_err());
    }
    for value in ["NaN", "inf", "-inf", "2147484", "bad"] {
        assert!(legacy_delay(&format!("[song]\ndelay_seconds = {value}\n")).is_err());
    }
}

#[test]
fn chart_normalization_preserves_other_sections_and_handles_missing_offset() {
    let original = Chart::parse(CHART.into()).unwrap();
    let normalized = apply_ini(
        Chart::parse(CHART.into()).unwrap(),
        "[song]\ndelay = -125\n",
    )
    .unwrap();
    assert_eq!(normalized.timing.offset, -0.125);
    assert!(normalized.raw.starts_with('\u{feff}'));
    assert!(normalized.raw.contains(" Custom = retain\r\n"));
    for before in original.sections.iter().filter(|s| s.name != "Song") {
        let after = normalized
            .sections
            .iter()
            .find(|s| s.name == before.name)
            .unwrap();
        assert_eq!(
            &original.raw[before.body_start..before.body_end],
            &normalized.raw[after.body_start..after.body_end]
        );
    }
    let raw = CHART.replace(" Offset = 0.5\r\n", "");
    let added = apply_ini(Chart::parse(raw).unwrap(), "[song]\ndelay_seconds = 0.0005").unwrap();
    assert_eq!(added.timing.offset, 0.0005);
    assert_eq!(added.raw.matches("Offset").count(), 1);
    let unchanged = apply_ini(Chart::parse(CHART.into()).unwrap(), "[song]\ndelay = 0").unwrap();
    assert_eq!(unchanged.raw, CHART);
}

#[test]
fn positive_and_negative_delays_survive_chart_and_bound_midi_reopen_exactly_once() {
    for millis in [250, -125] {
        let root = tempfile::tempdir().unwrap();
        let folder = song(root.path(), &millis.to_string());
        let original_ini = fs::read_to_string(folder.join("song.ini")).unwrap();
        let store = Store::create(&folder, None, &root.path().join("project")).unwrap();
        let expected = millis as f64 / 1000.;
        let project = store.load().unwrap();
        let clock = project.timing(&store).unwrap();
        assert_eq!(clock.offset, expected);
        assert!((clock.seconds(480) - (expected + 0.5)).abs() < 1e-12);
        assert_eq!(clock.tick(expected + 0.5), Some(480));
        let archive = store.root.join("source/.yace/originals/legacy-timing");
        assert_eq!(
            fs::read_to_string(archive.join("notes.chart")).unwrap(),
            CHART
        );
        assert_eq!(
            fs::read_to_string(archive.join("song.ini")).unwrap(),
            original_ini
        );
        let actor = Actor {
            kind: ActorKind::Human,
            name: "Delay roundtrip".into(),
        };
        let chart_output = root.path().join("chart-export");
        store
            .apply(Request {
                expected_revision: project.revision,
                actor: actor.clone(),
                action: Action::Export {
                    output: chart_output.clone(),
                },
            })
            .unwrap();
        let reopened =
            Store::create(&chart_output, None, &root.path().join("chart-reopened")).unwrap();
        assert_eq!(reopened.load().unwrap().timing(&reopened).unwrap(), clock);
        assert_eq!(
            crate::song_metadata::ini_fields(
                &fs::read_to_string(chart_output.join("song.ini")).unwrap()
            )["delay"],
            "0"
        );
        let vocals = VocalChart {
            version: 1,
            resolution: 480,
            source: "Timing fixture".into(),
            phrases: vec![VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            }],
            notes: vec![VocalNote {
                midi: None,
                start: 480,
                end: 720,
                tone: VocalTone::Pitched { midi: 60 },
                lyric: Syllable::Text {
                    text: "test".into(),
                    join_next: false,
                },
            }],
        };
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: actor.clone(),
                action: Action::SetVocals { document: vocals },
            })
            .unwrap();
        let midi_output = root.path().join("midi-export");
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor,
                action: Action::Export {
                    output: midi_output.clone(),
                },
            })
            .unwrap();
        assert_eq!(
            crate::song_metadata::ini_fields(
                &fs::read_to_string(midi_output.join("song.ini")).unwrap()
            )["delay"],
            millis.to_string()
        );
        let midi_reopened =
            Store::create(&midi_output, None, &root.path().join("midi-reopened")).unwrap();
        assert_eq!(
            midi_reopened
                .load()
                .unwrap()
                .timing(&midi_reopened)
                .unwrap(),
            clock
        );
        assert_eq!(
            fs::read_to_string(folder.join("notes.chart")).unwrap(),
            CHART
        );
        assert_eq!(
            fs::read_to_string(folder.join("song.ini")).unwrap(),
            original_ini
        );
    }
}

#[test]
fn enhancement_maps_audio_events_to_delayed_chart_and_midi_ticks() {
    for midi_input in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let folder = song(root.path(), if midi_input { "-125" } else { "50" });
        if midi_input {
            let tracks = Tracks::from([(
                "ExpertSingle".into(),
                vec![Note {
                    tick: 480,
                    lane: 0,
                    sustain: 120,
                }],
            )]);
            let bytes = crate::midi::new_midi(&TempoMap::constant(120.).unwrap(), &tracks).unwrap();
            fs::remove_file(folder.join("notes.chart")).unwrap();
            fs::write(folder.join("notes.mid"), bytes).unwrap();
        }
        let events = Transcription {
            drums: vec![DrumHit {
                time: 1.,
                voice: Voice::Kick,
                confidence: 0.9,
            }],
            ..Default::default()
        };
        let output = root.path().join("enhanced");
        pipeline::build(Build {
            audio: &folder.join("song.wav"),
            output: &output,
            existing: Some(&folder),
            timing: None,
            transcription: &events,
            meta: Metadata {
                name: "Test".into(),
                artist: "Test".into(),
                duration: 3.,
                ..Default::default()
            },
            pro: true,
            lookup_lyrics: false,
        })
        .unwrap();
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(crate::song_layout::path(
                &output,
                "chart-engine-report.json",
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["timing"]["offset"].as_f64().unwrap(),
            if midi_input { -0.125 } else { 0.05 }
        );
        if !midi_input {
            let chart =
                Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap()).unwrap();
            assert!(
                chart
                    .notes("ExpertDrums")
                    .unwrap()
                    .iter()
                    .any(|n| n.tick == 912)
            );
            assert_eq!(
                crate::song_metadata::ini_fields(
                    &fs::read_to_string(output.join("song.ini")).unwrap()
                )["delay"],
                "0"
            );
        } else {
            let parsed = crate::benchmark::parse(
                &fs::read(output.join("notes.mid")).unwrap(),
                crate::benchmark::Instrument::Drums,
                -125.,
            )
            .unwrap();
            assert!(parsed.notes.iter().any(|n| (n.start - 1.).abs() < 1e-9));
        }
    }
}

#[test]
fn invalid_delay_does_not_publish_a_project_or_change_input() {
    let root = tempfile::tempdir().unwrap();
    let folder = song(root.path(), "NaN");
    let output = root.path().join("project");
    assert!(Store::create(&folder, None, &output).is_err());
    assert!(!fs::exists(output));
    assert_eq!(
        fs::read_to_string(folder.join("notes.chart")).unwrap(),
        CHART
    );
}
