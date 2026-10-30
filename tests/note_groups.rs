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
        fs::read_to_string(output.join("editor-source/notes.chart")).unwrap(),
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
