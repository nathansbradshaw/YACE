use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    export_options::{Format, Options},
    note_groups::{self, Selection, Snapshot},
    note_types::Kind,
    six_fret,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Six-fret regression".into(),
    }
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: actor(),
            action,
        })
        .unwrap();
}
fn fixture(root: &Path, imported: bool) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    let mut text = String::from(
        "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n240 = N 0 0\n}\n",
    );
    if imported {
        for (_, suffix) in six_fret::PARTS {
            for tier in ["Easy", "Medium", "Hard", "Expert"] {
                text.push_str(&format!("[{tier}{suffix}]\n{{\n"));
                for (i, lane) in six_fret::ROWS.into_iter().enumerate() {
                    text.push_str(&format!("{} = N {lane} 120\n", (i + 1) * 240));
                }
                text.push_str(
                    "480 = N 6 0\n1440 = N 5 0\n0 = S 2 1920\n0 = E solo\n1920 = E soloend\n}\n",
                );
            }
        }
    }
    fs::write(song.join("notes.chart"), text).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Six fret regression\nsong_length = 4000\n",
    )
    .unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn midi_export(store: &Store, output: &Path) {
    fs::create_dir(output).unwrap();
    store
        .write_export_with_options(
            &store.load().unwrap(),
            output,
            &actor(),
            &Options {
                format: Format::Midi,
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
}

#[test]
fn twenty_tracks_create_all_white_black_open_lanes_and_keep_independent_history() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false);
    for (_, suffix) in six_fret::PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            apply(
                &store,
                Action::CreateTrack {
                    track: track.clone(),
                },
            );
            for (i, lane) in six_fret::ROWS.into_iter().enumerate() {
                apply(
                    &store,
                    Action::AddNote {
                        track: track.clone(),
                        tick: (i as u64 + 1) * 240,
                        lane,
                        sustain: 120,
                        cymbal: false,
                    },
                );
            }
            assert!(
                chart_engine::parts::Target::capture(&store, &store.load().unwrap(), &track)
                    .is_err()
            );
            apply(&store, Action::Undo);
            assert_eq!(store.load().unwrap().content.tracks[&track].len(), 6);
            apply(&store, Action::Redo);
            assert_eq!(store.load().unwrap().content.tracks[&track].len(), 7);
        }
    }
    let output = root.path().join("chart");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened"))
        .unwrap()
        .load()
        .unwrap();
    assert_eq!(reopened.content.tracks.len(), 21);
    for (_, suffix) in six_fret::PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            assert_eq!(reopened.content.tracks[&format!("{tier}{suffix}")].len(), 7);
        }
    }
}

#[test]
fn black_three_is_editable_typed_clipboard_isolated_and_midi_round_trip_preserves_all_parts() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let source = fs::read(store.root.join("source/notes.chart")).unwrap();
    let doc = store.load().unwrap();
    let track = "ExpertGHLGuitar";
    assert_eq!(doc.lane_rows(track), six_fret::ROWS);
    let id = doc.content.tracks[track]
        .iter()
        .find(|n| n.note.lane == 8)
        .unwrap()
        .id
        .clone();
    apply(
        &store,
        Action::UpdateNote {
            note_id: id.clone(),
            tick: 1441,
            lane: 8,
            sustain: 120,
            cymbal: false,
        },
    );
    let doc = store.load().unwrap();
    let selected = Selection {
        track: track.into(),
        ids: [id].into_iter().collect(),
        ..Default::default()
    };
    let snapshot = Snapshot::capture(&doc, &selected).unwrap();
    apply(
        &store,
        Action::SetNoteType {
            selection: snapshot,
            kind: Kind::Tap,
        },
    );
    let doc = store.load().unwrap();
    let clipboard = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selected).unwrap();
    assert!(
        clipboard
            .encode()
            .unwrap()
            .contains("\"family\":\"six_fret\"")
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    let error = store
        .apply(Request {
            expected_revision: doc.revision,
            actor: actor(),
            action: Action::EditNoteGroup {
                edit: note_groups::Edit::Paste {
                    track: "ExpertSingle".into(),
                    tick: 2000,
                    clipboard: clipboard.clone(),
                },
            },
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("compatible track"));
    assert_eq!(before, fs::read(store.root.join("project.json")).unwrap());
    apply(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Paste {
                track: "HardGHLBass".into(),
                tick: 2000,
                clipboard,
            },
        },
    );
    let doc = store.load().unwrap();
    assert!(
        doc.content.tracks["HardGHLBass"]
            .iter()
            .any(|n| n.note.lane == 8 && n.note.tick == 2000)
    );
    assert!(
        doc.content.tracks["HardGHLBass"]
            .iter()
            .any(|n| n.note.lane == 6 && n.note.tick == 2000)
    );
    let projected =
        chart_engine::octave_adapter::snapshot(&doc, &doc.timing(&store).unwrap()).unwrap();
    assert_eq!(
        projected
            .tracks
            .iter()
            .filter(|t| t.instrument == chart_engine::octave_adapter::Instrument::SixFret)
            .count(),
        20
    );
    let guitar = projected.tracks.iter().find(|t| t.key == track).unwrap();
    assert!(
        guitar
            .notes
            .iter()
            .any(|n| n.lane_number == 8 && n.lane_name == "Black 3" && n.flags.tap && n.editable)
    );
    let export = root.path().join("export");
    midi_export(&store, &export);
    let ordinary = root.path().join("ordinary");
    fs::create_dir(&ordinary).unwrap();
    for file in ["notes.mid", "song.ini"] {
        fs::copy(export.join(file), ordinary.join(file)).unwrap();
    }
    let bytes = fs::read(ordinary.join("notes.mid")).unwrap();
    let mut smf = midly::Smf::parse(&bytes).unwrap();
    for track in &mut smf.tracks {
        let ghl=track.iter().any(|e|matches!(e.kind,midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name)) if name.ends_with(b"GHL")));
        if !ghl {
            continue;
        }
        for event in track {
            if let midly::TrackEventKind::Midi { channel, message } = &mut event.kind {
                *channel = 3.into();
                match message {
                    midly::MidiMessage::NoteOn { vel, .. } if vel.as_int() > 0 => *vel = 83.into(),
                    midly::MidiMessage::NoteOff { vel, .. } => *vel = 7.into(),
                    _ => (),
                }
            }
        }
    }
    let mut custom = Vec::new();
    smf.write_std(&mut custom).unwrap();
    fs::write(ordinary.join("notes.mid"), &custom).unwrap();
    let reopened = Store::create(&ordinary, None, &root.path().join("midi-project")).unwrap();
    let loaded = reopened.load().unwrap();
    assert_eq!(loaded.content.tracks.len(), 21);
    for (_, suffix) in six_fret::PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let name = format!("{tier}{suffix}");
            let notes = &loaded.content.tracks[&name];
            assert!(
                six_fret::ROWS
                    .iter()
                    .all(|lane| notes.iter().any(|n| n.note.lane == *lane))
            );
            let chart =
                chart_engine::chart::Chart::parse(loaded.chart_text(&reopened).unwrap()).unwrap();
            assert_eq!(
                chart_engine::star_power::imported(&chart, &name).unwrap()[0].end,
                1920
            );
        }
    }
    let again = root.path().join("again");
    let black = loaded.content.tracks[track]
        .iter()
        .find(|n| n.note.lane == 8)
        .unwrap();
    assert_eq!(black.midi.as_ref().unwrap().channel, 3);
    apply(
        &reopened,
        Action::UpdateNote {
            note_id: black.id.clone(),
            tick: 1442,
            lane: 8,
            sustain: 120,
            cymbal: false,
        },
    );
    let edited = root.path().join("edited");
    midi_export(&reopened, &edited);
    let bytes = fs::read(edited.join("notes.mid")).unwrap();
    let midi = midly::Smf::parse(&bytes).unwrap();
    let guitar = midi
        .tracks
        .iter()
        .find(|t| {
            t.iter().any(|e| {
                matches!(
                    e.kind,
                    midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"PART GUITAR GHL"))
                )
            })
        })
        .unwrap();
    let mut tick = 0u64;
    assert!(guitar.iter().any(|e|{tick+=u64::from(e.delta.as_int()); tick==1442&&matches!(e.kind,midly::TrackEventKind::Midi{channel,message:midly::MidiMessage::NoteOn{key,vel}} if channel.as_int()==3&&key.as_int()==100&&vel.as_int()==83)}));
    apply(&reopened, Action::Undo);
    midi_export(&reopened, &again);
    assert_eq!(custom, fs::read(again.join("notes.mid")).unwrap());
    assert_eq!(
        source,
        fs::read(store.root.join("source/notes.chart")).unwrap()
    );
}

#[test]
fn old_chart_project_and_checkpoint_recover_all_twenty_tracks_without_rewriting_history() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let mut doc = store.load().unwrap();
    let original = doc.content.tracks.clone();
    doc.content
        .tracks
        .retain(|track, _| !six_fret::is_track(track));
    let checkpoint = chart_engine::project_recording::Checkpoint {
        content: doc.content.clone(),
        recording: doc.recording.clone(),
    }
    .encode()
    .unwrap();
    let id = format!("{:x}", Sha256::digest(&checkpoint));
    let path = store.root.join("checkpoints").join(format!("{id}.json"));
    fs::write(&path, &checkpoint).unwrap();
    doc.undo.push(id);
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .lane = 1;
    let before = serde_json::to_vec(&doc).unwrap();
    fs::write(store.root.join("project.json"), &before).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.content.tracks.len(), 21);
    for (track, notes) in original {
        if six_fret::is_track(&track) {
            assert_eq!(
                serde_json::to_value(&restored.content.tracks[&track]).unwrap(),
                serde_json::to_value(notes).unwrap()
            );
        }
    }
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(&store, Action::Undo);
    assert_eq!(store.load().unwrap().content.tracks.len(), 21);
    apply(&store, Action::Redo);
    assert_eq!(store.load().unwrap().content.tracks.len(), 21);
    assert_eq!(fs::read(path).unwrap(), checkpoint);
}
