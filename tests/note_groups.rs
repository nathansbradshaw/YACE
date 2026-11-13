#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    note_groups::{self, Clipboard, Edit, Selection, Snapshot},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use std::{collections::BTreeSet, fs};

fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
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
fn selected(store: &Store, track: &str, ticks: &[u64]) -> Selection {
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
        vocals: BTreeSet::new(),
    }
}
fn edit(store: &Store, edit: Edit) -> anyhow::Result<()> {
    apply(store, Action::EditNoteGroup { edit })
}
fn snap(store: &Store, selection: &Selection) -> Snapshot {
    Snapshot::capture(&store.load().unwrap(), selection).unwrap()
}
fn copied(store: &Store, selection: &Selection) -> Clipboard {
    let doc = store.load().unwrap();
    note_groups::copy(&doc, &doc.timing(store).unwrap(), selection).unwrap()
}
fn vocal_selection(indices: &[usize]) -> Selection {
    Selection {
        track: "Vocals".into(),
        vocals: indices.iter().copied().collect(),
        ..Default::default()
    }
}
fn vocals(store: &Store, notes: Vec<VocalNote>) {
    apply(
        store,
        Action::SetVocals {
            document: VocalChart {
                version: 1,
                resolution: 480,
                source: "Practice recording".into(),
                phrases: vec![VocalPhrase {
                    start: notes[0].start,
                    end: notes.last().unwrap().end,
                    player: 1,
                }],
                notes,
            },
        },
    )
    .unwrap();
}
fn syllable(start: u64, end: u64, lyric: Syllable) -> VocalNote {
    VocalNote {
        midi: None,
        start,
        end,
        tone: VocalTone::Pitched { midi: 60 },
        lyric,
    }
}
fn word(text: &str, join_next: bool) -> Syllable {
    Syllable::Text {
        text: text.into(),
        join_next,
    }
}

fn padded_vocal_fixture(root: &std::path::Path) -> Store {
    let store = support::fixture(root);
    apply(
        &store,
        Action::SetVocals {
            document: VocalChart {
                version: 1,
                resolution: 480,
                source: "Padded scoring phrases".into(),
                notes: vec![
                    syllable(480, 720, word("one", false)),
                    syllable(1440, 1680, word("two", false)),
                ],
                phrases: vec![
                    VocalPhrase {
                        start: 400,
                        end: 1000,
                        player: 1,
                    },
                    VocalPhrase {
                        start: 1200,
                        end: 2000,
                        player: 2,
                    },
                ],
            },
        },
    )
    .unwrap();
    store
}

#[test]
fn vocal_group_move_preserves_scoring_padding_and_unselected_phrase_edges() {
    let root = tempfile::tempdir().unwrap();
    let store = padded_vocal_fixture(root.path());
    let original = store.load().unwrap().content.vocals.unwrap();
    let selection = snap(&store, &vocal_selection(&[0]));
    edit(
        &store,
        Edit::Move {
            selection,
            ticks: 120,
            rows: 0,
        },
    )
    .unwrap();
    let moved = store.load().unwrap().content.vocals.unwrap();
    assert_eq!(
        moved.phrases[1], original.phrases[1],
        "Unselected scoring phrase lost its original boundaries"
    );
    assert_eq!(
        moved.phrases[0],
        VocalPhrase {
            start: 520,
            end: 1120,
            player: 1
        }
    );
    assert_eq!(moved.notes[1], original.notes[1]);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals.unwrap(), original);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals.unwrap(), moved);
}

#[test]
fn vocal_group_delete_keeps_the_remaining_phrase_padding() {
    let root = tempfile::tempdir().unwrap();
    let store = padded_vocal_fixture(root.path());
    let original = store.load().unwrap().content.vocals.unwrap();
    let selection = snap(&store, &vocal_selection(&[0]));
    edit(&store, Edit::Delete { selection }).unwrap();
    let remaining = store.load().unwrap().content.vocals.unwrap();
    assert_eq!(
        remaining.phrases,
        vec![original.phrases[1].clone()],
        "Deleting another phrase changed retained scoring boundaries"
    );
    assert_eq!(remaining.notes, vec![original.notes[1].clone()]);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals.unwrap(), original);
}

#[test]
fn partial_vocal_move_keeps_phrase_edges_and_preserves_unselected_notes() {
    let root = tempfile::tempdir().unwrap();
    let store = padded_vocal_fixture(root.path());
    let mut chart = store.load().unwrap().content.vocals.unwrap();
    chart
        .notes
        .insert(1, syllable(780, 900, word("extra", false)));
    apply(
        &store,
        Action::SetVocals {
            document: chart.clone(),
        },
    )
    .unwrap();
    let selection = snap(&store, &vocal_selection(&[0]));
    edit(
        &store,
        Edit::Move {
            selection,
            ticks: 30,
            rows: 0,
        },
    )
    .unwrap();
    let moved = store.load().unwrap().content.vocals.unwrap();
    assert_eq!(moved.phrases, chart.phrases);
    assert_eq!(&moved.notes[1..], &chart.notes[1..]);
    assert_eq!((moved.notes[0].start, moved.notes[0].end), (510, 750));
}

#[test]
fn vocal_group_refuses_overlapping_scoring_phrases_without_a_commit() {
    let root = tempfile::tempdir().unwrap();
    let store = padded_vocal_fixture(root.path());
    let mut chart = store.load().unwrap().content.vocals.unwrap();
    chart.phrases[1].player = 1;
    apply(&store, Action::SetVocals { document: chart }).unwrap();
    let selection = snap(&store, &vocal_selection(&[0]));
    let before = fs::read(store.root.join("project.json")).unwrap();
    let error = edit(
        &store,
        Edit::Move {
            selection,
            ticks: 360,
            rows: 0,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("scoring phrases"), "{error:#}");
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}

fn large_fixture(root: &std::path::Path) -> Store {
    use std::fmt::Write;
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    let mut text = String::from(
        "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n 0 = E \"section Large selection\"\n}\n[ExpertSingle]\n{\n 481 = N 1 0\n",
    );
    for i in 0..12_001u64 {
        let tick = 480 + 120 * i;
        writeln!(text, " {tick} = N {} 30", i % 3).unwrap();
        if i % 3 != 2 {
            writeln!(text, " {tick} = N {} 0", 5 + i % 3).unwrap();
        }
    }
    text.push_str("}\n");
    fs::write(source.join("notes.chart"), text).unwrap();
    fs::write(
        source.join("song.ini"),
        "[song]\nname = Large selection\nsong_length = 4000000\n",
    )
    .unwrap();
    let store = Store::new(root);
    store.initialize(&source, None, 4000.).unwrap();
    store
}
fn large_selection(store: &Store) -> Selection {
    let doc = store.load().unwrap();
    Selection {
        track: "ExpertSingle".into(),
        ids: doc.content.tracks["ExpertSingle"]
            .iter()
            .filter(|n| n.note.tick % 120 == 0 && doc.playable("ExpertSingle", n.note.lane))
            .map(|n| n.id.clone())
            .collect(),
        ..Default::default()
    }
}

#[test]
fn large_group_copy_move_paste_delete_and_history_keep_source_and_flags() {
    let root = tempfile::tempdir().unwrap();
    let store = large_fixture(root.path());
    let source = fs::read(root.path().join("source/notes.chart")).unwrap();
    let group = large_selection(&store);
    assert_eq!(group.len(), 12_001);
    let start = std::time::Instant::now();
    let clipboard = copied(&store, &group);
    let clipboard = Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
    let copied_in = start.elapsed();
    let original = store.load().unwrap();
    let start = std::time::Instant::now();
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &group),
            ticks: 60,
            rows: 0,
        },
    )
    .unwrap();
    let moved_in = start.elapsed();
    let moved = store.load().unwrap();
    assert_eq!(moved.undo.len(), 1);
    let after: std::collections::BTreeMap<_, _> = moved.content.tracks["ExpertSingle"]
        .iter()
        .map(|n| (&n.id, n))
        .collect();
    for note in &original.content.tracks["ExpertSingle"] {
        if group.ids.contains(&note.id) {
            let current = after[&note.id];
            assert_eq!(current.note.tick, note.note.tick + 60);
            assert_eq!(current.note.lane, note.note.lane);
            assert_eq!(current.note.sustain, note.note.sustain);
        }
    }
    let flags: BTreeSet<_> = moved.content.tracks["ExpertSingle"]
        .iter()
        .filter(|n| !moved.playable("ExpertSingle", n.note.lane))
        .map(|n| (n.note.tick, n.note.lane))
        .collect();
    for note in original.content.tracks["ExpertSingle"]
        .iter()
        .filter(|n| !original.playable("ExpertSingle", n.note.lane))
    {
        assert!(flags.contains(&(note.note.tick + 60, note.note.lane)));
    }
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(&store.load().unwrap().content).unwrap(),
        serde_json::to_value(&original.content).unwrap()
    );
    apply(&store, Action::Redo).unwrap();
    let start = std::time::Instant::now();
    edit(
        &store,
        Edit::Paste {
            track: "ExpertSingle".into(),
            tick: 2_000_000,
            clipboard,
        },
    )
    .unwrap();
    let pasted_in = start.elapsed();
    let pasted = store.load().unwrap();
    assert_eq!(
        pasted.content.tracks["ExpertSingle"]
            .iter()
            .filter(|n| pasted.playable("ExpertSingle", n.note.lane))
            .count(),
        24_003
    );
    let start = std::time::Instant::now();
    edit(
        &store,
        Edit::Delete {
            selection: snap(&store, &group),
        },
    )
    .unwrap();
    let deleted_in = start.elapsed();
    let deleted = store.load().unwrap();
    assert_eq!(
        deleted.content.tracks["ExpertSingle"]
            .iter()
            .filter(|n| deleted.playable("ExpertSingle", n.note.lane))
            .count(),
        12_002
    );
    assert_eq!(
        deleted.content.tracks["ExpertSingle"]
            .iter()
            .filter(|n| !deleted.playable("ExpertSingle", n.note.lane))
            .map(|n| n.note.tick)
            .min(),
        Some(2_000_000)
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(&store.load().unwrap().content).unwrap(),
        serde_json::to_value(pasted.content).unwrap()
    );
    assert_eq!(
        source,
        fs::read(root.path().join("source/notes.chart")).unwrap()
    );
    eprintln!(
        "large_group native debug: copy={copied_in:?} move={moved_in:?} paste={pasted_in:?} delete={deleted_in:?}"
    );
}

#[test]
fn large_group_destination_collision_leaves_committed_project_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = large_fixture(root.path());
    let group = large_selection(&store);
    let before = fs::read(root.path().join("project.json")).unwrap();
    let result = edit(
        &store,
        Edit::Move {
            selection: snap(&store, &group),
            ticks: 1,
            rows: 1,
        },
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Nothing was changed")
    );
    assert_eq!(before, fs::read(root.path().join("project.json")).unwrap());
    assert!(store.load().unwrap().undo.is_empty());
}

#[test]
fn oversized_bulk_requests_and_clipboard_bytes_refuse_without_committing() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let group = selected(&store, "ExpertSingle", &[480]);
    let mut clipboard = copied(&store, &group);
    if let note_groups::Payload::Instrument { notes, .. } = &mut clipboard.payload {
        notes.resize(100_001, notes[0].clone());
    }
    assert!(
        clipboard
            .encode()
            .unwrap_err()
            .to_string()
            .contains("100,000")
    );
    let untrusted = format!(
        "YACE notes v1\n{}",
        serde_json::to_string(&clipboard).unwrap()
    );
    assert!(
        Clipboard::decode(&untrusted)
            .unwrap_err()
            .to_string()
            .contains("100,000")
    );
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(
        edit(
            &store,
            Edit::Paste {
                track: "ExpertSingle".into(),
                tick: 0,
                clipboard
            }
        )
        .unwrap_err()
        .to_string()
        .contains("100,000")
    );
    assert_eq!(before, fs::read(root.path().join("project.json")).unwrap());
    assert!(
        Clipboard::decode(&"x".repeat(32 * 1024 * 1024 + 1))
            .unwrap_err()
            .to_string()
            .contains("32 MiB")
    );
}

#[test]
fn large_vocal_groups_keep_independent_phrases_and_undoable_words() {
    let root = tempfile::tempdir().unwrap();
    let store = large_fixture(root.path());
    let notes: Vec<_> = (0..12_001u64)
        .map(|i| syllable(480 + i * 120, 540 + i * 120, word("ah", false)))
        .collect();
    let phrases = notes
        .iter()
        .map(|n| VocalPhrase {
            start: n.start,
            end: n.end,
            player: 1,
        })
        .collect();
    apply(
        &store,
        Action::SetVocals {
            document: VocalChart {
                version: 1,
                resolution: 480,
                source: "Controlled bulk vocal fixture".into(),
                notes,
                phrases,
            },
        },
    )
    .unwrap();
    let selection = Selection {
        track: "Vocals".into(),
        vocals: (0..12_001).collect(),
        ..Default::default()
    };
    let before = store.load().unwrap();
    let start = std::time::Instant::now();
    let clipboard = copied(&store, &selection);
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 48,
            rows: 1,
        },
    )
    .unwrap();
    let moved = store.load().unwrap();
    let chart = moved.content.vocals.as_ref().unwrap();
    assert_eq!(chart.phrases.len(), 12_001);
    assert_eq!(chart.notes[0].start, 528);
    assert_eq!(chart.notes[0].tone, VocalTone::Pitched { midi: 59 });
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, before.content.vocals);
    apply(&store, Action::Redo).unwrap();
    edit(
        &store,
        Edit::Paste {
            track: "Vocals".into(),
            tick: 2_000_000,
            clipboard,
        },
    )
    .unwrap();
    assert_eq!(
        store
            .load()
            .unwrap()
            .content
            .vocals
            .as_ref()
            .unwrap()
            .notes
            .len(),
        24_002
    );
    edit(
        &store,
        Edit::Delete {
            selection: snap(&store, &selection),
        },
    )
    .unwrap();
    let remaining = store.load().unwrap();
    let chart = remaining.content.vocals.as_ref().unwrap();
    assert_eq!(chart.notes.len(), 12_001);
    assert_eq!(chart.phrases.len(), 12_001);
    assert_eq!(chart.notes[0].start, 2_000_000);
    eprintln!(
        "large_vocal_group native debug complete copy/move/UndoRedo/paste/delete={:?}",
        start.elapsed()
    );
}

#[test]
fn moving_a_group_is_one_undo_and_preserves_original_sources() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let before = store.load().unwrap();
    let chart = fs::read(root.path().join("source/notes.chart")).unwrap();
    let audio = fs::read(root.path().join("source/song.ogg")).unwrap();
    let selection = selected(&store, "ExpertSingle", &[480, 500]);
    let ids = selection.ids.clone();
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 20,
            rows: 1,
        },
    )
    .unwrap();
    let moved = store.load().unwrap();
    assert_eq!(moved.undo.len(), 1);
    for id in ids {
        let old = before.find_note(&id).unwrap().1;
        let now = moved.find_note(&id).unwrap().1;
        assert_eq!(now.note.tick, old.note.tick + 20);
        assert_eq!(now.note.lane, old.note.lane + 1);
    }
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
    apply(&store, Action::Redo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(moved.content).unwrap()
    );
    assert_eq!(
        fs::read(root.path().join("source/notes.chart")).unwrap(),
        chart
    );
    assert_eq!(
        fs::read(root.path().join("source/song.ogg")).unwrap(),
        audio
    );
}

#[test]
fn collisions_and_bounds_fail_without_a_partial_edit_or_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    for (ticks, rows) in [(1900, 1), (-501, 0), (0, -1), (4000, 0)] {
        let selection = selected(&store, "ExpertSingle", &[480, 500]);
        let bytes = fs::read(root.path().join("project.json")).unwrap();
        assert!(
            edit(
                &store,
                Edit::Move {
                    selection: snap(&store, &selection),
                    ticks,
                    rows
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), bytes);
        assert_eq!(
            fs::read_dir(root.path().join("checkpoints"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn clipboard_roundtrip_pastes_fresh_ids_and_keeps_spacing() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let selection = selected(&store, "ExpertSingle", &[480, 500]);
    let clipboard = Clipboard::decode(&copied(&store, &selection).encode().unwrap()).unwrap();
    edit(
        &store,
        Edit::Paste {
            track: "ExpertSingle".into(),
            tick: 960,
            clipboard,
        },
    )
    .unwrap();
    let after = store.load().unwrap();
    let notes = &after.content.tracks["ExpertSingle"];
    assert!(
        notes
            .iter()
            .any(|n| n.note.tick == 960 && !selection.ids.contains(&n.id))
    );
    assert!(notes.iter().any(|n| n.note.tick == 980));
    assert_eq!(after.undo.len(), 1);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        3
    );
}

#[test]
fn delete_is_atomic_and_stale_gestures_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let selection = selected(&store, "ExpertSingle", &[480, 500]);
    let stale = snap(&store, &selection);
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 2,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    assert!(edit(&store, Edit::Delete { selection: stale }).is_err());
    edit(
        &store,
        Edit::Delete {
            selection: snap(&store, &selection),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        2
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        4
    );
}

#[test]
fn paste_scales_sustains_and_allows_guitar_to_bass_but_rejects_drums() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    apply(
        &store,
        Action::CreateTrack {
            track: "ExpertDoubleBass".into(),
        },
    )
    .unwrap();
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 2,
            sustain: 240,
            cymbal: false,
        },
    )
    .unwrap();
    let mut clipboard = copied(&store, &selected(&store, "ExpertSingle", &[960]));
    clipboard.resolution = 960;
    edit(
        &store,
        Edit::Paste {
            track: "ExpertDoubleBass".into(),
            tick: 1440,
            clipboard: clipboard.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertDoubleBass"][0]
            .note
            .sustain,
        120
    );
    assert!(
        edit(
            &store,
            Edit::Paste {
                track: "ExpertDrums".into(),
                tick: 1440,
                clipboard
            }
        )
        .is_err()
    );
}

#[test]
fn drum_markers_follow_their_gem_and_do_not_affect_other_lanes() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    apply(
        &store,
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 960,
            lane: 2,
            sustain: 0,
            cymbal: true,
        },
    )
    .unwrap();
    let mut doc = store.load().unwrap();
    doc.content
        .tracks
        .get_mut("ExpertDrums")
        .unwrap()
        .push(chart_engine::editor::EditorNote {
            midi: None,
            id: "accent".into(),
            note: chart_engine::chart::Note {
                tick: 960,
                lane: 35,
                sustain: 0,
            },
        });
    support::save(&store, &doc);
    let selection = selected(&store, "ExpertDrums", &[960]);
    let clipboard = copied(&store, &selection);
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 480,
            rows: 1,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let notes = &doc.content.tracks["ExpertDrums"];
    for lane in [3, 67, 36] {
        assert!(
            notes
                .iter()
                .any(|n| n.note.tick == 1440 && n.note.lane == lane)
        );
    }
    assert!(!notes.iter().any(|n| n.note.tick == 960));
    edit(
        &store,
        Edit::Paste {
            track: "ExpertDrums".into(),
            tick: 1920,
            clipboard,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    assert!(
        doc.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 1920 && n.note.lane == 66)
    );
}

#[test]
fn shared_fretted_controls_stay_on_the_remaining_chord_and_follow_the_copy() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut doc = store.load().unwrap();
    let notes = doc.content.tracks.get_mut("ExpertSingle").unwrap();
    for (id, lane) in [("chord", 1), ("forced", 5)] {
        notes.push(chart_engine::editor::EditorNote {
            midi: None,
            id: id.into(),
            note: chart_engine::chart::Note {
                tick: 480,
                lane,
                sustain: 0,
            },
        });
    }
    support::save(&store, &doc);
    let selection = Selection {
        track: "ExpertSingle".into(),
        ids: BTreeSet::from(["chord".into()]),
        ..Default::default()
    };
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 480,
            rows: 0,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    for tick in [480, 960] {
        assert!(
            doc.content.tracks["ExpertSingle"]
                .iter()
                .any(|n| n.note.tick == tick && n.note.lane == 5)
        );
    }
}

#[test]
fn untrusted_clipboards_reject_plain_text_unknown_fields_and_bad_flags() {
    assert!(Clipboard::decode("hello").is_err());
    assert!(Clipboard::decode("YACE notes v1\n{\"version\":2,\"resolution\":480,\"payload\":{\"kind\":\"instrument\",\"family\":\"frets\",\"notes\":[]}}").is_err());
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut clipboard = copied(&store, &selected(&store, "ExpertSingle", &[480]));
    if let note_groups::Payload::Instrument { notes, .. } = &mut clipboard.payload {
        notes[0].flags.push(66);
    }
    assert!(
        edit(
            &store,
            Edit::Paste {
                track: "ExpertSingle".into(),
                tick: 960,
                clipboard
            }
        )
        .is_err()
    );
}

#[test]
fn vocal_words_require_complete_selection_and_keep_missing_lyrics() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    vocals(
        &store,
        vec![
            syllable(100, 200, word("hel", true)),
            syllable(240, 300, word("lo", false)),
            syllable(340, 400, Syllable::ContinueWord),
            syllable(600, 700, Syllable::Missing),
        ],
    );
    let doc = store.load().unwrap();
    let partial = vocal_selection(&[0, 2]);
    assert!(note_groups::copy(&doc, &doc.timing(&store).unwrap(), &partial).is_err());
    assert!(
        edit(
            &store,
            Edit::Delete {
                selection: snap(&store, &vocal_selection(&[1]))
            }
        )
        .is_err()
    );
    let selection = vocal_selection(&[0, 1, 2]);
    let clipboard = copied(&store, &selection);
    edit(
        &store,
        Edit::Paste {
            track: "Vocals".into(),
            tick: 1000,
            clipboard,
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.vocals.as_ref().unwrap().notes[3].lyric,
        Syllable::Missing
    );
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 20,
            rows: -2,
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.vocals.as_ref().unwrap().notes[0].tone,
        VocalTone::Pitched { midi: 62 }
    );
}

#[test]
fn paste_cannot_steal_an_existing_vocal_continuation() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    vocals(
        &store,
        vec![
            syllable(100, 150, word("stay", false)),
            syllable(300, 350, Syllable::Continue),
            syllable(600, 650, word("new", false)),
        ],
    );
    let clipboard = copied(&store, &vocal_selection(&[2]));
    let before = store.load().unwrap();
    assert!(
        edit(
            &store,
            Edit::Paste {
                track: "Vocals".into(),
                tick: 200,
                clipboard
            }
        )
        .is_err()
    );
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
}

#[test]
fn deleting_all_vocals_can_be_undone_in_one_step() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    vocals(
        &store,
        vec![
            syllable(100, 200, Syllable::Missing),
            syllable(300, 400, Syllable::Missing),
        ],
    );
    let before = store.load().unwrap();
    let selection = vocal_selection(&[0, 1]);
    edit(
        &store,
        Edit::Delete {
            selection: snap(&store, &selection),
        },
    )
    .unwrap();
    assert!(store.load().unwrap().content.vocals.is_none());
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
}

#[test]
fn expert_group_edits_refresh_linked_difficulties_and_undo_together() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    apply(
        &store,
        Action::CreateTrack {
            track: "HardSingle".into(),
        },
    )
    .unwrap();
    apply(
        &store,
        Action::EnableDifficultyLink {
            track: "HardSingle".into(),
            enabled: true,
        },
    )
    .unwrap();
    let before = store.load().unwrap();
    let selection = selected(&store, "ExpertSingle", &[480, 500, 2400]);
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 480,
            rows: 0,
        },
    )
    .unwrap();
    let after = store.load().unwrap();
    assert_ne!(
        before.content.tracks["HardSingle"],
        after.content.tracks["HardSingle"]
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(before.content).unwrap()
    );
}

#[test]
fn pasted_groups_export_with_the_original_timing_and_vocal_uncertainty() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let clipboard = copied(&store, &selected(&store, "ExpertSingle", &[480, 500]));
    edit(
        &store,
        Edit::Paste {
            track: "ExpertSingle".into(),
            tick: 960,
            clipboard,
        },
    )
    .unwrap();
    vocals(&store, vec![syllable(100, 200, Syllable::Missing)]);
    let clipboard = copied(&store, &vocal_selection(&[0]));
    edit(
        &store,
        Edit::Paste {
            track: "Vocals".into(),
            tick: 960,
            clipboard,
        },
    )
    .unwrap();
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let chart = chart_engine::chart::Chart::parse(
        fs::read_to_string(output.join(".yace/history/editor-source/notes.chart")).unwrap(),
    )
    .unwrap();
    assert!(
        chart
            .notes("ExpertSingle")
            .unwrap()
            .iter()
            .any(|n| n.tick == 980)
    );
    assert_eq!(chart.timing.resolution, 480);
    let midi = fs::read(output.join("notes.mid")).unwrap();
    let smf = midly::Smf::parse(&midi).unwrap();
    assert!(smf.tracks.iter().any(|track| {
        let mut tick = 0;
        track.iter().any(|event| {
            tick += event.delta.as_int() as u64;
            tick == 980 && matches!(event.kind, midly::TrackEventKind::Midi { message:midly::MidiMessage::NoteOn { key,vel },.. } if key.as_int()==96 && vel.as_int()>0)
        })
    }));
    let vocals = chart_engine::vocals::import_midi(&midi).unwrap();
    assert_eq!(vocals.notes.len(), 2);
    assert!(vocals.notes.iter().all(|n| n.lyric == Syllable::Missing));
}

#[test]
fn piano_clipboards_keep_five_lane_and_pitched_parts_distinct() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    for (track, lane) in [("ExpertKeyboard", 2), ("ExpertProKeys", 60)] {
        apply(
            &store,
            Action::CreateTrack {
                track: track.into(),
            },
        )
        .unwrap();
        apply(
            &store,
            Action::AddNote {
                track: track.into(),
                tick: 480,
                lane,
                sustain: 120,
                cymbal: false,
            },
        )
        .unwrap();
        let selection = selected(&store, track, &[480]);
        let clipboard = copied(&store, &selection);
        edit(
            &store,
            Edit::Paste {
                track: track.into(),
                tick: 960,
                clipboard: clipboard.clone(),
            },
        )
        .unwrap();
        assert!(
            store.load().unwrap().content.tracks[track]
                .iter()
                .any(|n| n.note.tick == 960 && n.note.lane == lane && n.note.sustain == 120)
        );
        assert!(
            edit(
                &store,
                Edit::Paste {
                    track: "ExpertSingle".into(),
                    tick: 1440,
                    clipboard
                }
            )
            .is_err()
        );
    }
    let selection = selected(&store, "ExpertProKeys", &[480, 960]);
    edit(
        &store,
        Edit::Move {
            selection: snap(&store, &selection),
            ticks: 120,
            rows: 2,
        },
    )
    .unwrap();
    assert!(
        store.load().unwrap().content.tracks["ExpertProKeys"]
            .iter()
            .all(|n| n.note.lane == 58)
    );
}
