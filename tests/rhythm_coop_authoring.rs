use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    export_options::{Format, Options},
    parts,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Authoring regression".into(),
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
    let mut source = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n 0 = E \"section retained\"\n}\n[ExpertSingle]\n{\n 240 = N 0 0\n}\n".to_owned();
    if imported {
        for (_, suffix) in parts::MANUAL_PARTS {
            for tier in ["Easy", "Medium", "Hard", "Expert"] {
                source.push_str(&format!("[{tier}{suffix}]\n{{\n 480 = N 1 240\n 480 = N 5 0\n 960 = N 2 0\n 960 = N 6 0\n 0 = S 2 1440\n 0 = E solo\n 1440 = E soloend\n}}\n"));
            }
        }
    }
    fs::write(song.join("notes.chart"), source).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Authoring regression\nsong_length = 4000\n",
    )
    .unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}

#[test]
fn both_manual_parts_can_be_created_at_every_tier_without_exposing_a_generator() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false);
    let original = fs::read(store.root.join("source/notes.chart")).unwrap();
    for (_, suffix) in parts::MANUAL_PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            apply(
                &store,
                Action::CreateTrack {
                    track: track.clone(),
                },
            );
            apply(
                &store,
                Action::AddNote {
                    track: track.clone(),
                    tick: 720,
                    lane: 2,
                    sustain: 240,
                    cymbal: false,
                },
            );
            assert!(parts::Target::capture(&store, &store.load().unwrap(), &track).is_err());
            apply(&store, Action::Undo);
            assert!(store.load().unwrap().content.tracks[&track].is_empty());
            apply(&store, Action::Redo);
            assert_eq!(
                store.load().unwrap().content.tracks[&track][0].note.sustain,
                240
            );
        }
    }
    let output = root.path().join("export");
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
    assert_eq!(reopened.content.tracks.len(), 9);
    assert_eq!(reopened.content.tracks["ExpertSingle"][0].note.lane, 0);
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        original
    );
}

#[test]
fn imported_parts_expose_each_tier_and_retain_modifiers_phrases_and_other_parts() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let original = fs::read(store.root.join("source/notes.chart")).unwrap();
    for (_, suffix) in parts::MANUAL_PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            let doc = store.load().unwrap();
            let id = doc.content.tracks[&track]
                .iter()
                .find(|n| n.note.lane == 1)
                .unwrap()
                .id
                .clone();
            apply(
                &store,
                Action::SelectTrack {
                    track: track.clone(),
                },
            );
            apply(
                &store,
                Action::UpdateNote {
                    note_id: id.clone(),
                    tick: 481,
                    lane: 3,
                    sustain: 240,
                    cymbal: false,
                },
            );
            let notes = store.load().unwrap().content.tracks[&track].clone();
            assert!(notes.iter().any(|n| n.note.tick == 481 && n.note.lane == 5));
            assert!(notes.iter().any(|n| n.note.tick == 960 && n.note.lane == 6));
            let doc = store.load().unwrap();
            let selection = chart_engine::note_groups::Snapshot::capture(
                &doc,
                &chart_engine::note_groups::Selection {
                    track: track.clone(),
                    ids: [id].into_iter().collect(),
                    ..Default::default()
                },
            )
            .unwrap();
            apply(
                &store,
                Action::SetNoteType {
                    selection,
                    kind: chart_engine::note_types::Kind::Tap,
                },
            );
            assert!(
                store.load().unwrap().content.tracks[&track]
                    .iter()
                    .any(|n| n.note.tick == 481 && n.note.lane == 6)
            );
            apply(&store, Action::Undo);
        }
    }
    let doc = store.load().unwrap();
    let snapshot =
        chart_engine::octave_adapter::snapshot(&doc, &doc.timing(&store).unwrap()).unwrap();
    assert_eq!(snapshot.tracks.len(), 9);
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let chart = Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap()).unwrap();
    for (_, suffix) in parts::MANUAL_PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            assert_eq!(
                chart_engine::star_power::imported(&chart, &track).unwrap()[0].end,
                1440
            );
            assert!(
                chart
                    .notes(&track)
                    .unwrap()
                    .iter()
                    .any(|n| n.tick == 481 && n.lane == 3 && n.sustain == 240)
            );
        }
    }
    let text = fs::read_to_string(output.join("notes.chart")).unwrap();
    assert_eq!(text.matches("E soloend").count(), 8);
    assert_eq!(chart.notes("ExpertSingle").unwrap()[0].lane, 0);
    let midi_dir = root.path().join("midi-export");
    fs::create_dir(&midi_dir).unwrap();
    store
        .write_export_with_options(
            &doc,
            &midi_dir,
            &actor(),
            &Options {
                format: Format::Midi,
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    let ordinary = root.path().join("ordinary-midi");
    fs::create_dir(&ordinary).unwrap();
    for name in ["notes.mid", "song.ini"] {
        fs::copy(midi_dir.join(name), ordinary.join(name)).unwrap();
    }
    // Exercise nondefault note attributes, not just default MIDI data which is
    // intentionally represented by None in the editor.
    let bytes = fs::read(ordinary.join("notes.mid")).unwrap();
    let mut midi = midly::Smf::parse(&bytes).unwrap();
    for track in &mut midi.tracks {
        let retained_part = track.iter().any(|event| {
            matches!(
                event.kind,
                midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(
                    b"PART RHYTHM" | b"PART GUITAR COOP"
                ))
            )
        });
        if !retained_part {
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
    midi.write_std(&mut custom).unwrap();
    fs::write(ordinary.join("notes.mid"), custom).unwrap();
    let imported_store =
        Store::create(&ordinary, None, &root.path().join("midi-reopened")).unwrap();
    let imported = imported_store.load().unwrap();
    for (_, suffix) in parts::MANUAL_PARTS {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            let track = format!("{tier}{suffix}");
            assert!(
                imported.content.tracks[&track]
                    .iter()
                    .any(|n| n.note.tick == 481
                        && n.note.lane == 3
                        && n.note.sustain == 240
                        && n.midi.is_some()),
                "{track}: {:?}",
                imported.content.tracks[&track]
            );
        }
    }
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        original
    );
}

#[test]
fn legacy_project_and_undo_materialize_preserved_tracks_without_rewriting_evidence() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true);
    let mut doc = store.load().unwrap();
    let original_tracks = doc.content.tracks.clone();
    doc.content
        .tracks
        .retain(|track, _| track == "ExpertSingle");
    let checkpoint = chart_engine::project_recording::Checkpoint {
        content: doc.content.clone(),
        recording: doc.recording.clone(),
    }
    .encode()
    .unwrap();
    let checkpoint_id = format!("{:x}", Sha256::digest(&checkpoint));
    let checkpoint_path = store
        .root
        .join("checkpoints")
        .join(format!("{checkpoint_id}.json"));
    fs::write(&checkpoint_path, &checkpoint).unwrap();
    doc.undo.push(checkpoint_id);
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .lane = 1;
    let encoded = serde_json::to_vec(&doc).unwrap();
    fs::write(store.root.join("project.json"), &encoded).unwrap();
    let loaded = Store::new(&store.root).load().unwrap();
    assert_eq!(loaded.content.tracks.len(), 9);
    for (track, notes) in &original_tracks {
        if track != "ExpertSingle" {
            assert_eq!(
                serde_json::to_value(&loaded.content.tracks[track]).unwrap(),
                serde_json::to_value(notes).unwrap()
            );
        }
    }
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), encoded);
    apply(&store, Action::Undo);
    assert_eq!(store.load().unwrap().content.tracks.len(), 9);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"][0]
            .note
            .lane,
        0
    );
    apply(&store, Action::Redo);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"][0]
            .note
            .lane,
        1
    );
    assert_eq!(fs::read(checkpoint_path).unwrap(), checkpoint);
}
