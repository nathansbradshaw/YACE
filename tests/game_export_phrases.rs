use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    game_export, models,
    vocals::{self, Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use std::fs;

fn chart(parts: &str) -> Chart {
    Chart::parse(format!(
        "[Song]\n{{\n Resolution = 480\n}}\n[SyncTrack]\n{{\n 0 = B 120000\n 480 = B 90000\n}}\n[Events]\n{{\n}}\n{parts}"
    ))
    .unwrap()
}

fn vocal() -> VocalChart {
    VocalChart {
        version: 1,
        resolution: 480,
        source: "Authored test fixture".into(),
        phrases: vec![VocalPhrase {
            start: 0,
            end: 960,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 480,
            end: 720,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "test".into(),
                join_next: false,
            },
        }],
    }
}

fn spans(bytes: &[u8], name: &str, key: u8) -> Vec<(u64, u64)> {
    let smf = Smf::parse(bytes).unwrap();
    let track = smf
        .tracks
        .iter()
        .find(|track| {
            track.iter().any(|event| {
                matches!(event.kind, TrackEventKind::Meta(MetaMessage::TrackName(value)) if value == name.as_bytes())
            })
        })
        .unwrap();
    let mut tick = 0;
    let mut active = None;
    let mut result = Vec::new();
    for event in track {
        tick += u64::from(event.delta.as_int());
        match event.kind {
            TrackEventKind::Midi {
                message: MidiMessage::NoteOn { key: note, vel },
                ..
            } if note.as_int() == key && vel.as_int() > 0 => {
                assert!(
                    active.replace(tick).is_none(),
                    "overlapping marker note-ons"
                );
            }
            TrackEventKind::Midi {
                message: MidiMessage::NoteOff { key: note, .. },
                ..
            } if note.as_int() == key => result.push((active.take().unwrap(), tick)),
            _ => {}
        }
    }
    assert!(active.is_none());
    result
}

#[test]
fn solo_markers_export_for_each_supported_instrument_with_pro_keys() {
    let source = chart(
        "[ExpertSingle]\n{\n 0 = N 0 0\n 120 = E solo\n 720 = E soloend\n}\n\
         [ExpertDoubleBass]\n{\n 0 = N 1 0\n 240 = E \"solo\"\n 600 = E \"soloend\"\n}\n\
         [ExpertDrums]\n{\n 0 = N 1 0\n 0 = E solo\n 480 = E soloend\n}\n\
         [ExpertKeyboard]\n{\n 0 = N 2 0\n 480 = E soloend\n 0 = E solo\n}\n\
         [ExpertProKeys]\n{\n 0 = N 60 120\n 120 = E solo\n 720 = E soloend\n}\n",
    );
    let (bytes, _) = game_export::song(&source, Some(&vocal())).unwrap();
    assert_eq!(spans(&bytes, "PART GUITAR", 103), [(120, 720)]);
    assert_eq!(spans(&bytes, "PART BASS", 103), [(240, 600)]);
    assert_eq!(spans(&bytes, "PART DRUMS", 103), [(0, 480)]);
    assert_eq!(spans(&bytes, "PART KEYS", 103), [(0, 480)]);
    assert_eq!(spans(&bytes, "PART REAL_KEYS_X", 115), [(120, 720)]);
    assert_eq!(vocals::import_midi(&bytes).unwrap().notes, vocal().notes);
}

#[test]
fn shared_solos_union_across_difficulties_without_duplicate_note_ons() {
    let source = chart(
        "[EasySingle]\n{\n 0 = N 0 0\n 0 = E solo\n 480 = E soloend\n}\n\
         [HardSingle]\n{\n 0 = N 1 0\n 240 = E solo\n 720 = E soloend\n}\n\
         [ExpertSingle]\n{\n 0 = N 2 0\n 720 = E solo\n 960 = E soloend\n 1200 = E solo\n 1440 = E soloend\n}\n",
    );
    let bytes = game_export::song(&source, None).unwrap().0;
    assert_eq!(
        spans(&bytes, "PART GUITAR", 103),
        [(0, 720), (720, 960), (1200, 1440)]
    );
}

#[test]
fn malformed_or_unknown_local_events_remain_explicit_export_errors() {
    for events in [
        "0 = E solo",
        "480 = E soloend",
        "0 = E solo\n0 = E soloend",
        "0 = E solo\n120 = E solo\n480 = E soloend",
        "0 = E custom_event",
    ] {
        let source = chart(&format!("[ExpertSingle]\n{{\n0 = N 0 0\n{events}\n}}\n"));
        assert!(game_export::song(&source, None).is_err(), "{events}");
    }
}

#[test]
fn vocal_song_export_reopens_with_solos_and_unchanged_source() {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    let source = chart("[ExpertSingle]\n{\n0 = N 0 120\n120 = E solo\n720 = E soloend\n}\n");
    fs::write(song.join("notes.chart"), &source.raw).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Solo fixture\ncustom = retain\n",
    )
    .unwrap();
    let samples = vec![0.; 44100 * 3];
    models::write_audio(&song.join("song.wav"), &samples, &samples).unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let actor = Actor {
        kind: ActorKind::Human,
        name: "Solo export fixture".into(),
    };
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: actor.clone(),
            action: Action::SetVocals { document: vocal() },
        })
        .unwrap();
    let output = root.path().join("export");
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: actor.clone(),
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    assert_eq!(spans(&bytes, "PART GUITAR", 103), [(120, 720)]);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    assert_eq!(doc.chart_text(&reopened).unwrap(), source.raw);
    assert_eq!(doc.content.vocals.unwrap().notes, vocal().notes);
    assert_eq!(
        fs::read_to_string(song.join("notes.chart")).unwrap(),
        source.raw
    );
    assert!(
        fs::read_to_string(output.join("song.ini"))
            .unwrap()
            .contains("custom = retain")
    );
}
