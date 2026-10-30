#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::{Chart, Note},
    editor::{Action, Actor, ActorKind, DecisionStatus, EditorNote, Request, Store},
    note_groups::{self, Selection, Snapshot},
    note_types::{self, Kind},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
fn action(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "You".into(),
        },
        action,
    })?;
    Ok(())
}
fn selection(store: &Store, track: &str, ticks: &[u64]) -> Selection {
    let doc = store.load().unwrap();
    Selection {
        track: track.into(),
        ids: doc.content.tracks[track]
            .iter()
            .filter(|n| {
                ticks.contains(&n.note.tick) && chart_engine::editor::playable(track, n.note.lane)
            })
            .map(|n| n.id.clone())
            .collect(),
        ..Default::default()
    }
}
fn set(store: &Store, track: &str, ticks: &[u64], kind: Kind) -> anyhow::Result<()> {
    let snapshot = Snapshot::capture(&store.load()?, &selection(store, track, ticks))?;
    action(
        store,
        Action::SetNoteType {
            selection: snapshot,
            kind,
        },
    )
}
fn playable(store: &Store) -> BTreeMap<String, Vec<EditorNote>> {
    store
        .load()
        .unwrap()
        .content
        .tracks
        .into_iter()
        .map(|(track, notes)| {
            let notes = notes
                .into_iter()
                .filter(|n| chart_engine::editor::playable(&track, n.note.lane))
                .collect();
            (track, notes)
        })
        .collect()
}
fn add(store: &Store, tick: u64, lane: u8) {
    action(
        store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick,
            lane,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
}
#[test]
fn one_gem_changes_the_whole_chord_and_one_undo_restores_it() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    add(&store, 480, 2);
    let before = playable(&store);
    let original = fs::read(root.path().join("source/notes.chart")).unwrap();
    let mut selected = selection(&store, "ExpertSingle", &[480]);
    selected.ids = selected.ids.into_iter().take(1).collect();
    let snapshot = Snapshot::capture(&store.load().unwrap(), &selected).unwrap();
    let undo = store.load().unwrap().undo.len();
    action(
        &store,
        Action::SetNoteType {
            selection: snapshot,
            kind: Kind::Tap,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    assert_eq!(doc.undo.len(), undo + 1);
    assert_eq!(
        doc.content.tracks["ExpertSingle"]
            .iter()
            .filter(|n| n.note.tick == 480 && n.note.lane == 6)
            .count(),
        1
    );
    let notes = doc.content.tracks["ExpertSingle"]
        .iter()
        .map(|n| n.note.clone())
        .collect::<Vec<_>>();
    let policy = chart_engine::gameplay_view::yarg_chart_policy(480, &BTreeMap::new()).unwrap();
    let gems = chart_engine::gameplay_view::classify(
        "ExpertSingle",
        &notes,
        &doc.timing(&store).unwrap(),
        &policy,
    )
    .unwrap();
    assert!(
        gems.iter()
            .filter(|g| g.tick == 480)
            .all(|g| g.technique == Some(chart_engine::gameplay_view::Technique::Tap))
    );
    let content = serde_json::to_value(doc.content).unwrap();
    action(&store, Action::Undo).unwrap();
    assert!(
        store.load().unwrap().content.tracks["ExpertSingle"]
            .iter()
            .all(|n| n.note.lane != 6)
    );
    action(&store, Action::Redo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        content
    );
    assert_eq!(playable(&store), before);
    assert_eq!(
        fs::read(root.path().join("source/notes.chart")).unwrap(),
        original
    );
}
#[test]
fn forced_flips_both_natural_types_and_auto_removes_only_selected_flags() {
    use chart_engine::gameplay_view::{self, Technique};
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    add(&store, 600, 1);
    set(&store, "ExpertSingle", &[600, 2400], Kind::Forced).unwrap();
    let doc = store.load().unwrap();
    let notes = doc.content.tracks["ExpertSingle"]
        .iter()
        .map(|n| n.note.clone())
        .collect::<Vec<_>>();
    let policy = gameplay_view::yarg_chart_policy(480, &BTreeMap::new()).unwrap();
    let gems = gameplay_view::classify(
        "ExpertSingle",
        &notes,
        &doc.timing(&store).unwrap(),
        &policy,
    )
    .unwrap();
    assert_eq!(
        gems.iter().find(|g| g.tick == 600).unwrap().technique,
        Some(Technique::Strum)
    );
    assert_eq!(
        gems.iter().find(|g| g.tick == 2400).unwrap().technique,
        Some(Technique::Hopo)
    );
    set(&store, "ExpertSingle", &[600], Kind::Tap).unwrap();
    set(&store, "ExpertSingle", &[600], Kind::Auto).unwrap();
    let doc = store.load().unwrap();
    let markers = note_types::markers(&doc.content.tracks["ExpertSingle"]);
    assert!(!markers.contains_key(&600));
    assert_eq!(markers[&2400], Kind::Forced);
}
#[test]
fn stale_incompatible_empty_and_overflowing_edits_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let old = Snapshot::capture(
        &store.load().unwrap(),
        &selection(&store, "ExpertSingle", &[480]),
    )
    .unwrap();
    add(&store, 600, 1);
    let before = fs::read(root.path().join("project.json")).unwrap();
    let checkpoints = fs::read_dir(root.path().join("checkpoints"))
        .unwrap()
        .count();
    let drum = Snapshot::capture(
        &store.load().unwrap(),
        &selection(&store, "ExpertDrums", &[480]),
    )
    .unwrap();
    let mut empty = Snapshot::capture(
        &store.load().unwrap(),
        &selection(&store, "ExpertSingle", &[480]),
    )
    .unwrap();
    empty.ids.clear();
    for selection in [old, drum, empty] {
        assert!(
            action(
                &store,
                Action::SetNoteType {
                    selection,
                    kind: Kind::Tap
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    }
    assert_eq!(
        fs::read_dir(root.path().join("checkpoints"))
            .unwrap()
            .count(),
        checkpoints
    );
    let mut doc = store.load().unwrap();
    doc.next_id = u64::MAX;
    support::save(&store, &doc);
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(set(&store, "ExpertSingle", &[480], Kind::Tap).is_err());
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
}
#[test]
fn imported_ids_and_foreign_flags_remain_safe() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut doc = store.load().unwrap();
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0].id = "edit-note-1".into();
    doc.content
        .tracks
        .get_mut("ExpertSingle")
        .unwrap()
        .push(EditorNote {
            id: "foreign".into(),
            note: Note {
                tick: 480,
                lane: 12,
                sustain: 17,
            },
        });
    let selected = Selection {
        track: "ExpertSingle".into(),
        ids: BTreeSet::from(["edit-note-1".into()]),
        ..Default::default()
    };
    let snapshot = Snapshot::capture(&doc, &selected).unwrap();
    note_types::apply(&mut doc, &snapshot, Kind::Forced).unwrap();
    assert!(
        doc.content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.id == "foreign" && n.note.sustain == 17)
    );
    assert!(
        doc.content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.id == "edit-note-2" && n.note.lane == 5)
    );
}
#[test]
fn note_type_changes_stale_reviews_and_undo_restores_them() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let region = store.load().unwrap().review.regions[0].id.clone();
    action(
        &store,
        Action::Decide {
            region_id: region.clone(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "Checked against the recording".into(),
        },
    )
    .unwrap();
    set(&store, "ExpertSingle", &[480], Kind::Tap).unwrap();
    let doc = store.load().unwrap();
    assert!(
        doc.decision(&region, "ExpertSingle", &doc.timing(&store).unwrap())
            .is_none()
    );
    action(&store, Action::Undo).unwrap();
    let doc = store.load().unwrap();
    assert!(
        doc.decision(&region, "ExpertSingle", &doc.timing(&store).unwrap())
            .is_some()
    );
}
#[test]
fn clipboard_moves_and_linked_difficulties_keep_note_types() {
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
    set(&store, "ExpertSingle", &[480], Kind::Tap).unwrap();
    let doc = store.load().unwrap();
    let selected = selection(&store, "ExpertSingle", &[480]);
    let clip = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selected).unwrap();
    let snapshot = Snapshot::capture(&doc, &selected).unwrap();
    action(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Move {
                selection: snapshot,
                ticks: 960,
                rows: 1,
            },
        },
    )
    .unwrap();
    assert_eq!(
        note_types::markers(&store.load().unwrap().content.tracks["ExpertSingle"])[&1440],
        Kind::Tap
    );
    action(
        &store,
        Action::EditNoteGroup {
            edit: note_groups::Edit::Paste {
                track: "ExpertSingle".into(),
                tick: 3000,
                clipboard: clip,
            },
        },
    )
    .unwrap();
    assert_eq!(
        note_types::markers(&store.load().unwrap().content.tracks["ExpertSingle"])[&3000],
        Kind::Tap
    );
    let linked = store.load().unwrap();
    assert!(linked.content.difficulty_links.links["HardSingle"].enabled);
}

fn midi_events(bytes: &[u8]) -> (Vec<(u64, u8)>, Vec<(u64, Vec<u8>)>) {
    let smf = midly::Smf::parse(bytes).unwrap();
    let mut ons = vec![];
    let mut sysex = vec![];
    for track in &smf.tracks {
        let mut tick = 0;
        for event in track {
            tick += event.delta.as_int() as u64;
            match event.kind {
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { key, vel },
                    ..
                } if vel.as_int() > 0 => ons.push((tick, key.as_int())),
                midly::TrackEventKind::SysEx(data) => sysex.push((tick, data.to_vec())),
                _ => {}
            }
        }
    }
    (ons, sysex)
}
#[test]
fn midi_taps_are_point_ranges_scoped_to_one_difficulty_and_open_notes_survive() {
    let raw = "[Song]\n{\nResolution = 480\n}\n[ExpertSingle]\n{\n480 = N 0 0\n480 = N 5 0\n480 = N 6 0\n481 = N 1 0\n}\n[HardSingle]\n{\n480 = N 0 0\n480 = N 5 0\n}\n[EasyDoubleBass]\n{\n960 = N 7 0\n}\n";
    let chart = Chart::parse(raw.into()).unwrap();
    let bytes = chart_engine::game_export::chart_to_midi(&chart).unwrap();
    let (ons, sysex) = midi_events(&bytes);
    assert_eq!(
        sysex,
        vec![
            (480, vec![b'P', b'S', 0, 0, 3, 4, 1, 0xf7]),
            (480, vec![b'P', b'S', 0, 0, 3, 4, 0, 0xf7])
        ]
    );
    assert!(ons.contains(&(480, 89)));
    assert!(ons.contains(&(481, 101)));
    assert!(ons.contains(&(960, 59)));
    assert!(!ons.contains(&(480, 104)));
    assert!(!ons.contains(&(480, 101)));
    let smf = midly::Smf::parse(&bytes).unwrap();
    assert!(smf.tracks.iter().flatten().any(|e| matches!(
        e.kind,
        midly::TrackEventKind::Meta(midly::MetaMessage::Text(b"[ENHANCED_OPENS]"))
    )));
}
#[test]
fn midi_resolves_the_chart_policy_and_preserves_editor_metadata() {
    let raw = "[Song]\n{\nResolution = 480\n}\n[ExpertSingle]\n{\n100 = N 0 0\n250 = N 1 0\n250 = N 5 0\n}\n";
    let chart = Chart::parse(raw.into()).unwrap();
    let (default, _) = chart_engine::game_export::song(&chart, None).unwrap();
    assert!(midi_events(&default).0.contains(&(250, 102)));
    let ini = BTreeMap::from([("hopo_frequency".into(), "100".into())]);
    let (custom, _) = chart_engine::game_export::song_with_ini(&chart, None, &ini).unwrap();
    assert!(midi_events(&custom).0.contains(&(250, 101)));
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    fs::write(
        root.path().join("source/song.ini"),
        "[song]\nhopo_frequency = 100\n",
    )
    .unwrap();
    add(&store, 650, 1);
    set(&store, "ExpertSingle", &[650], Kind::Forced).unwrap();
    action(
        &store,
        Action::CreateTrack {
            track: "ExpertProKeys".into(),
        },
    )
    .unwrap();
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let exported = fs::read(output.join("notes.mid")).unwrap();
    assert!(midi_events(&exported).0.contains(&(650, 101)));
    assert!(
        fs::read_to_string(output.join("song.ini"))
            .unwrap()
            .contains("hopo_frequency = 100")
    );
}
