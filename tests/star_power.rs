#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    star_power::{self, Edit, Phrase},
};
use std::{collections::BTreeMap, fs};
fn phrase(start: u64, end: u64) -> Phrase {
    Phrase { start, end }
}
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "You".into(),
    }
}
fn action(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: actor(),
        action,
    })?;
    Ok(())
}
fn edit(store: &Store, edit: Edit) -> anyhow::Result<()> {
    action(store, Action::EditStarPower { edit })
}
fn put(store: &Store, track: &str, previous: Option<Phrase>, range: Phrase) -> anyhow::Result<()> {
    edit(
        store,
        Edit::Put {
            track: track.into(),
            previous,
            phrase: range,
        },
    )
}
fn text(store: &Store) -> String {
    store.load().unwrap().chart_text(store).unwrap()
}

#[test]
fn ranges_add_move_resize_delete_and_restore_as_single_undo_steps() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let original = fs::read(root.path().join("source/notes.chart")).unwrap();
    let notes = store.load().unwrap().content.tracks;
    put(&store, "ExpertSingle", None, phrase(480, 960)).unwrap();
    assert_eq!(store.load().unwrap().undo.len(), 1);
    put(
        &store,
        "ExpertSingle",
        Some(phrase(480, 960)),
        phrase(960, 1920),
    )
    .unwrap();
    assert!(text(&store).contains("960 = S 2 960"));
    let edited = serde_json::to_value(store.load().unwrap().content).unwrap();
    edit(
        &store,
        Edit::Remove {
            track: "ExpertSingle".into(),
            phrase: phrase(960, 1920),
        },
    )
    .unwrap();
    assert!(!text(&store).contains(" = S 2 "));
    action(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        edited
    );
    action(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().content.tracks, notes);
    assert_eq!(
        fs::read(root.path().join("source/notes.chart")).unwrap(),
        original
    );
}
#[test]
fn invalid_stale_overlapping_and_out_of_recording_ranges_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    put(&store, "ExpertSingle", None, phrase(480, 960)).unwrap();
    let before = fs::read(root.path().join("project.json")).unwrap();
    let checkpoints = fs::read_dir(root.path().join("checkpoints"))
        .unwrap()
        .count();
    for edit in [
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(960, 960),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(961, 960),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(480, 800),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(0, 800),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(900, 1200),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: None,
            phrase: phrase(4000, 5000),
        },
        Edit::Put {
            track: "Vocals".into(),
            previous: None,
            phrase: phrase(0, 100),
        },
        Edit::Put {
            track: "Missing".into(),
            previous: None,
            phrase: phrase(0, 100),
        },
        Edit::Put {
            track: "ExpertSingle".into(),
            previous: Some(phrase(0, 100)),
            phrase: phrase(0, 200),
        },
        Edit::Remove {
            track: "ExpertSingle".into(),
            phrase: phrase(480, 961),
        },
    ] {
        assert!(self::edit(&store, edit).is_err());
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.path().join("checkpoints"))
                .unwrap()
                .count(),
            checkpoints
        );
    }
    put(&store, "ExpertSingle", None, phrase(960, 1200)).unwrap();
}
#[test]
fn source_phrases_import_sorted_and_other_events_and_crlf_survive() {
    let raw = "[Song]\r\n{\r\n Resolution = 480\r\n}\r\n[ExpertSingle]\r\n{\r\n // keep\r\n 960 = S 2 480\r\n 0 = E solo\r\n 480 = N 0 0\r\n 0 = S 2 240\r\n 1500 = S 64 10\r\n 1600 = E soloend\r\n}\r\n[ExpertDrums]\r\n{\r\n 0 = S 2 100\r\n}\r\n";
    let chart = Chart::parse(raw.into()).unwrap();
    assert_eq!(
        star_power::imported(&chart, "ExpertSingle").unwrap(),
        vec![phrase(0, 240), phrase(960, 1440)]
    );
    assert_eq!(
        star_power::render(raw.into(), &BTreeMap::new()).unwrap(),
        raw
    );
    let changed = star_power::render(
        raw.into(),
        &BTreeMap::from([("ExpertSingle".into(), vec![phrase(480, 960)])]),
    )
    .unwrap();
    assert_eq!(
        star_power::imported(&Chart::parse(changed.clone()).unwrap(), "ExpertSingle").unwrap(),
        vec![phrase(480, 960)]
    );
    for retained in [
        " // keep\r\n",
        " 0 = E solo\r\n",
        " 480 = N 0 0\r\n",
        " 1500 = S 64 10\r\n",
        " 1600 = E soloend\r\n",
        "[ExpertDrums]\r\n{\r\n 0 = S 2 100\r\n}\r\n",
    ] {
        assert!(changed.contains(retained));
    }
    let cleared = star_power::render(
        raw.into(),
        &BTreeMap::from([("ExpertSingle".into(), vec![])]),
    )
    .unwrap();
    assert!(
        star_power::imported(&Chart::parse(cleared).unwrap(), "ExpertSingle")
            .unwrap()
            .is_empty()
    );
}
#[test]
fn malformed_source_is_not_silently_discarded() {
    for line in [
        "0 = S 2 0",
        "0 = S 2 bad",
        "0 = S 2 10 extra",
        "18446744073709551615 = S 2 5",
        "0 = S 2 20\n10 = S 2 30",
    ] {
        let raw = format!("[Song]\n{{\n}}\n[ExpertSingle]\n{{\n{line}\n}}\n");
        let chart = Chart::parse(raw.clone()).unwrap();
        assert!(star_power::imported(&chart, "ExpertSingle").is_err());
        assert_eq!(
            star_power::render(raw.clone(), &BTreeMap::new()).unwrap(),
            raw
        );
    }
}
#[test]
fn selected_notes_cover_sustains_and_include_the_last_gem() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut doc = store.load().unwrap();
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .sustain = 1800;
    let selection = chart_engine::note_groups::Selection {
        track: "ExpertSingle".into(),
        ids: doc.content.tracks["ExpertSingle"]
            .iter()
            .map(|n| n.id.clone())
            .collect(),
        ..Default::default()
    };
    assert_eq!(
        star_power::selected(&doc, &selection, 120).unwrap(),
        phrase(480, 2520)
    );
    let mut stale = selection.clone();
    stale.ids.insert("removed-note".into());
    assert!(star_power::selected(&doc, &stale, 120).is_err());
    assert!(star_power::selected(&doc, &Default::default(), 120).is_err());
}
#[test]
fn old_projects_default_and_piano_parts_export_their_own_phrases() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut value = serde_json::to_value(store.load().unwrap()).unwrap();
    value["content"]
        .as_object_mut()
        .unwrap()
        .remove("star_power");
    fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(store.load().unwrap().content.star_power.is_empty());
    for track in ["ExpertKeyboard", "ExpertProKeys"] {
        action(
            &store,
            Action::CreateTrack {
                track: track.into(),
            },
        )
        .unwrap();
        put(&store, track, None, phrase(480, 960)).unwrap();
    }
    let chart = Chart::parse(text(&store)).unwrap();
    for track in ["ExpertKeyboard", "ExpertProKeys"] {
        assert_eq!(
            star_power::imported(&chart, track).unwrap(),
            vec![phrase(480, 960)]
        );
    }
    let bytes = chart_engine::game_export::chart_to_midi(&chart).unwrap();
    let midi = midly::Smf::parse(&bytes).unwrap();
    assert_eq!(midi.tracks.iter().flatten().filter(|e|matches!(e.kind,midly::TrackEventKind::Midi{message:midly::MidiMessage::NoteOn{key,..},..} if key.as_int()==116)).count(),2);
}
#[test]
fn midi_unions_shared_difficulty_ranges_without_duplicate_note_ons() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    action(
        &store,
        Action::CreateTrack {
            track: "HardSingle".into(),
        },
    )
    .unwrap();
    put(&store, "ExpertSingle", None, phrase(480, 1440)).unwrap();
    put(&store, "HardSingle", None, phrase(960, 1920)).unwrap();
    let chart = Chart::parse(text(&store)).unwrap();
    let bytes = chart_engine::game_export::chart_to_midi(&chart).unwrap();
    let midi = midly::Smf::parse(&bytes).unwrap();
    let mut actual = vec![];
    for track in &midi.tracks {
        let mut tick = 0;
        for event in track {
            tick += event.delta.as_int() as u64;
            if let midly::TrackEventKind::Midi { message, .. } = event.kind {
                match message {
                    midly::MidiMessage::NoteOn { key, .. } if key.as_int() == 116 => {
                        actual.push((tick, true))
                    }
                    midly::MidiMessage::NoteOff { key, .. } if key.as_int() == 116 => {
                        actual.push((tick, false))
                    }
                    _ => {}
                }
            }
        }
    }
    assert_eq!(actual, vec![(480, true), (1920, false)]);
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(output.join("notes.chart")).unwrap(),
        chart.raw
    );
}
#[test]
fn tempo_changes_cannot_push_authored_phrases_past_the_recording() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    put(&store, "ExpertSingle", None, phrase(3000, 3800)).unwrap();
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(
        action(
            &store,
            Action::EditSync {
                edit: chart_engine::sync_track::Edit::PutTempo {
                    previous: Some(chart_engine::timing::Tempo { tick: 0, bpm: 120. }),
                    tempo: chart_engine::timing::Tempo { tick: 0, bpm: 100. }
                }
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
}

#[test]
fn linked_note_updates_keep_independent_phrase_ranges() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    action(
        &store,
        Action::CreateTrack {
            track: "HardSingle".into(),
        },
    )
    .unwrap();
    action(
        &store,
        Action::EnableDifficultyLink {
            track: "HardSingle".into(),
            enabled: true,
        },
    )
    .unwrap();
    put(&store, "ExpertSingle", None, phrase(480, 960)).unwrap();
    put(&store, "HardSingle", None, phrase(0, 400)).unwrap();
    let before = store.load().unwrap().content.star_power;
    action(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 3000,
            lane: 2,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    assert_eq!(store.load().unwrap().content.star_power, before);
    action(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.star_power, before);
}
