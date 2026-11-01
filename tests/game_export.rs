use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    game_export, midi, models,
    vocals::{self, Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use std::fs;

const CHART: &str = "[Song]\n{\n Resolution = 480\n Offset = -0.01\n}\n[SyncTrack]\n{\n 0 = TS 4\n 0 = B 120000\n 960 = TS 3 3\n 960 = B 90000\n}\n[Events]\n{\n 0 = E \"section intro\"\n}\n[ExpertSingle]\n{\n 480 = N 0 240\n}\n[ExpertDoubleBass]\n{\n 720 = N 1 0\n}\n[EasyDrums]\n{\n 480 = N 2 0\n}\n[ExpertDrums]\n{\n 480 = N 2 240\n 960 = N 3 0\n 960 = N 67 0\n 480 = S 2 960\n}\n";
fn vocal() -> VocalChart {
    VocalChart {
        version: 1,
        resolution: 480,
        source: "Known test pitch and authored timing".into(),
        phrases: vec![VocalPhrase {
            start: 400,
            end: 1500,
            player: 1,
        }],
        notes: vec![
            VocalNote {
                start: 480,
                end: 720,
                tone: VocalTone::Pitched { midi: 60 },
                lyric: Syllable::Text {
                    text: "hold".into(),
                    join_next: false,
                },
            },
            VocalNote {
                start: 1080,
                end: 1400,
                tone: VocalTone::Pitched { midi: 62 },
                lyric: Syllable::Text {
                    text: "near".into(),
                    join_next: false,
                },
            },
        ],
    }
}
#[test]
fn vocal_game_export_keeps_ticks_tempos_signatures_events_and_exact_offset() {
    let chart = Chart::parse(CHART.into()).unwrap();
    let (bytes, delay) = game_export::song_with_vocals(&chart, &vocal()).unwrap();
    assert_eq!(delay, -10);
    let timing = midi::timing(&bytes).unwrap();
    for tick in [0, 480, 960, 1080, 1400] {
        assert!(
            (timing.seconds(tick) + delay as f64 / 1000. - chart.timing.seconds(tick)).abs()
                < 0.000001
        );
    }
    assert_eq!(vocals::import_midi(&bytes).unwrap().notes, vocal().notes);
    let inventory = midi::inventory(&bytes).unwrap();
    assert!(
        inventory
            .iter()
            .any(|(name, count)| name == "PART BASS" && *count == 1)
    );
    assert!(
        inventory
            .iter()
            .any(|(name, count)| name == "PART GUITAR" && *count == 1)
    );
    let smf = midly::Smf::parse(&bytes).unwrap();
    assert!(smf.tracks[0].iter().any(|e| matches!(
        e.kind,
        midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(3, 3, 24, 8))
    )));
    assert!(smf.tracks[1].iter().any(|e| matches!(
        e.kind,
        midly::TrackEventKind::Meta(midly::MetaMessage::Text(b"[section intro]"))
    )));
    // The two overlapping yellow tom markers from different difficulties are
    // unioned: one start, one end, with no nested or prematurely ended marker.
    let markers: Vec<_> = smf
        .tracks
        .iter()
        .flatten()
        .filter_map(|e| match e.kind {
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { key, vel },
                ..
            } if key.as_int() == 110 && vel.as_int() > 0 => Some(true),
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOff { key, .. },
                ..
            } if key.as_int() == 110 => Some(false),
            _ => None,
        })
        .collect();
    assert_eq!(markers, vec![true, false]);
    assert!(game_export::with_vocals(&chart, &vocal()).is_err());
    let ini = game_export::ini_with_delay(
        "[song]\r\nname = Keep\r\ndelay = 0\r\ndelay_seconds = 0\r\ncustom = yes\r\n[other]\r\ndelay = 9\r\n",
        delay,
    );
    assert!(ini.contains("custom = yes\r\n"));
    assert!(ini.contains("[other]\r\ndelay = 9"));
    assert!(!ini.contains("delay_seconds"));
    assert_eq!(ini.matches("delay = -10").count(), 1);
}
#[test]
fn unsupported_chart_semantics_are_refused_instead_of_lost() {
    for raw in [
        CHART.replace("480 = N 0 240", "480 = N 0 240\n 481 = N 5 0"),
        CHART.replace("[ExpertSingle]", "[ExpertDoubleGuitar]"),
        CHART.replace("Offset = -0.01", "Offset = 0.0005"),
    ] {
        assert!(game_export::song_with_vocals(&Chart::parse(raw).unwrap(), &vocal()).is_err());
    }
}
#[test]
fn same_editor_action_edits_vocals_undoes_and_exports_one_playable_song() {
    let temp = tempfile::tempdir().unwrap();
    let song = temp.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Vocal test\ncustom = keep\n",
    )
    .unwrap();
    let samples = vec![0.; 44100 * 4];
    models::write_audio(&song.join("song.wav"), &samples, &samples).unwrap();
    let audio_hash = models::hash_file(&song.join("song.wav")).unwrap();
    let store = Store::create(&song, None, &temp.path().join("project")).unwrap();
    let apply = |action| {
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Vocal test".into(),
                },
                action,
            })
            .unwrap()
    };
    apply(Action::SetVocals { document: vocal() });
    let instrument_text = store.load().unwrap().chart_text(&store).unwrap();
    assert_eq!(instrument_text, CHART);
    apply(Action::Undo);
    assert!(store.load().unwrap().content.vocals.is_none());
    apply(Action::Redo);
    assert_eq!(store.load().unwrap().content.vocals.unwrap(), vocal());
    let export = temp.path().join("game");
    apply(Action::Export {
        output: export.clone(),
    });
    assert!(export.join("notes.mid").exists());
    assert!(!export.join("notes.chart").exists());
    assert_eq!(
        fs::read_to_string(export.join("editor-source/notes.chart")).unwrap(),
        CHART
    );
    assert_eq!(
        models::hash_file(&export.join("song.wav")).unwrap(),
        audio_hash
    );
    assert!(
        fs::read_to_string(export.join("song.ini"))
            .unwrap()
            .contains("delay = -10")
    );
    assert_eq!(
        vocals::import_midi(&fs::read(export.join("notes.mid")).unwrap())
            .unwrap()
            .notes,
        vocal().notes
    );
    assert_eq!(fs::read_to_string(song.join("notes.chart")).unwrap(), CHART);
    let invalid = VocalChart {
        resolution: 192,
        ..vocal()
    };
    let revision = store.load().unwrap().revision;
    assert!(
        store
            .apply(Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Test".into()
                },
                action: Action::SetVocals { document: invalid }
            })
            .is_err()
    );
    assert_eq!(store.load().unwrap().revision, revision);
    fs::write(store.root.join("source/song.ini"), [0xff, 0xfe]).unwrap();
    let refused = temp.path().join("refused-unreadable-metadata");
    assert!(
        store
            .apply(Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Test".into()
                },
                action: Action::Export {
                    output: refused.clone()
                }
            })
            .is_err()
    );
    assert!(!refused.exists());
}
