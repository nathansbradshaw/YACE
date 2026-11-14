#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::Note,
    editor::{Action, Actor, ActorKind, EditorNote, Request, Store},
    note_groups::{self, Edit, Selection, Snapshot},
    snap_grid::RepairPolicy,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use std::{collections::BTreeSet, fs};

fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Repair regression".into(),
        },
        action,
    })?;
    Ok(())
}
fn gem(tick: u64, lane: u8, sustain: u64) -> EditorNote {
    EditorNote {
        id: format!("fixture-{tick}-{lane}"),
        midi: None,
        note: Note {
            tick,
            lane,
            sustain,
        },
    }
}
fn seeded(notes: Vec<EditorNote>) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut project = store.load().unwrap();
    project.content.tracks.insert("ExpertSingle".into(), notes);
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    (root, store)
}
fn whole(store: &Store, track: &str) -> Snapshot {
    let p = store.load().unwrap();
    Snapshot::capture(&p, &note_groups::repair::whole_part(&p, track).unwrap()).unwrap()
}
fn policy(tolerance: Option<u64>) -> RepairPolicy {
    RepairPolicy {
        divisions: 4,
        maximum_shift: tolerance,
    }
}

#[test]
fn preview_is_read_only_and_near_policy_preserves_off_grid_lengths_ids_flags_and_history() {
    let mut first = gem(243, 0, 77);
    first.midi = Some(chart_engine::song_files::game_midi::NoteAttributes {
        channel: 5,
        velocity: 89,
        off_velocity: 17,
        off_as_note_on: false,
        zero_length: false,
        talkie_pitch: None,
    });
    let (_root, store) = seeded(vec![first, gem(243, 5, 0), gem(660, 1, 31), gem(960, 2, 0)]);
    let before = store.load().unwrap();
    let bytes = fs::read(store.root.join("project.json")).unwrap();
    let source = fs::read(store.root.join("source/notes.chart")).unwrap();
    let selection = whole(&store, "ExpertSingle");
    let report = note_groups::repair::preview(
        &before,
        &before.timing(&store).unwrap(),
        &selection,
        policy(Some(5)),
    )
    .unwrap();
    assert_eq!(
        (
            report.examined,
            report.changed,
            report.already_on_grid,
            report.kept_off_grid,
            report.largest_shift
        ),
        (3, 1, 1, 1, 3)
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), bytes);
    apply(
        &store,
        Action::EditNoteGroup {
            edit: Edit::Repair {
                selection,
                policy: policy(Some(5)),
            },
        },
    )
    .unwrap();
    let repaired = store.load().unwrap();
    let after = &repaired.content.tracks["ExpertSingle"];
    let original = &before.content.tracks["ExpertSingle"][0];
    let moved = after.iter().find(|n| n.id == original.id).unwrap();
    assert_eq!(
        (moved.note.tick, moved.note.sustain, moved.midi.clone()),
        (240, 77, original.midi.clone())
    );
    assert!(after.iter().any(|n| n.note.tick == 240 && n.note.lane == 5));
    for tick in [660, 960] {
        assert!(
            after.contains(
                before.content.tracks["ExpertSingle"]
                    .iter()
                    .find(|n| n.note.tick == tick)
                    .unwrap()
            )
        );
    }
    assert_eq!(
        repaired.content.tracks["ExpertDrums"],
        before.content.tracks["ExpertDrums"]
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(&before.content).unwrap()
    );
    apply(&store, Action::Redo).unwrap();
    assert_eq!(
        serde_json::to_value(store.load().unwrap().content).unwrap(),
        serde_json::to_value(&repaired.content).unwrap()
    );
    assert_eq!(
        fs::read(store.root.join("source/notes.chart")).unwrap(),
        source
    );
}

#[test]
fn preview_and_apply_refuse_collisions_and_incompatible_attack_flags_atomically() {
    for notes in [
        vec![gem(239, 0, 0), gem(241, 0, 0)],
        vec![gem(239, 0, 0), gem(239, 5, 0), gem(241, 1, 0)],
    ] {
        let (_root, store) = seeded(notes);
        let p = store.load().unwrap();
        let selection = whole(&store, "ExpertSingle");
        let before = fs::read(store.root.join("project.json")).unwrap();
        assert!(
            note_groups::repair::preview(&p, &p.timing(&store).unwrap(), &selection, policy(None))
                .is_err()
        );
        assert!(
            apply(
                &store,
                Action::EditNoteGroup {
                    edit: Edit::Repair {
                        selection,
                        policy: policy(None)
                    }
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    }
}

#[test]
fn unchanged_policy_and_stale_preview_never_commit() {
    let (_root, store) = seeded(vec![gem(243, 0, 0)]);
    let selection = whole(&store, "ExpertSingle");
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        apply(
            &store,
            Action::EditNoteGroup {
                edit: Edit::Repair {
                    selection: selection.clone(),
                    policy: policy(Some(2))
                }
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(
        &store,
        Action::EditNoteGroup {
            edit: Edit::Move {
                selection: selection.clone(),
                ticks: 1,
                rows: 0,
            },
        },
    )
    .unwrap();
    let moved = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        apply(
            &store,
            Action::EditNoteGroup {
                edit: Edit::Repair {
                    selection,
                    policy: policy(None)
                }
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), moved);
}

fn vocal(start: u64, text: &str) -> VocalNote {
    VocalNote {
        start,
        end: start + 30,
        tone: VocalTone::Pitched { midi: 60 },
        lyric: Syllable::Text {
            text: text.into(),
            join_next: false,
        },
        midi: None,
    }
}
#[test]
fn vocal_repairs_keep_phrase_padding_attributes_unselected_ranges_and_word_ownership() {
    for second in [363, 365] {
        let (_root, store) = seeded(vec![gem(480, 0, 0)]);
        let attributes = chart_engine::song_files::game_midi::NoteAttributes {
            channel: 3,
            velocity: 77,
            off_velocity: 19,
            off_as_note_on: false,
            zero_length: false,
            talkie_pitch: None,
        };
        let chart = VocalChart {
            version: 1,
            resolution: 480,
            source: "Supplied syllables".into(),
            notes: vec![
                vocal(243, "one"),
                vocal(second, "two"),
                vocal(1440, "three"),
            ],
            phrases: vec![
                VocalPhrase {
                    start: 200,
                    end: 470,
                    player: 1,
                    midi: Some(attributes),
                },
                VocalPhrase {
                    start: 1200,
                    end: 2000,
                    player: 2,
                    midi: None,
                },
            ],
        };
        apply(
            &store,
            Action::SetVocals {
                document: chart.clone(),
            },
        )
        .unwrap();
        let p = store.load().unwrap();
        let selection = whole(&store, "Vocals");
        let report = note_groups::repair::preview(
            &p,
            &p.timing(&store).unwrap(),
            &selection,
            policy(Some(5)),
        )
        .unwrap();
        assert_eq!(report.changed, 2);
        apply(
            &store,
            Action::EditNoteGroup {
                edit: Edit::Repair {
                    selection,
                    policy: policy(Some(5)),
                },
            },
        )
        .unwrap();
        let after = store.load().unwrap().content.vocals.unwrap();
        assert_eq!(
            after
                .notes
                .iter()
                .map(|n| (n.start, n.end - n.start))
                .collect::<Vec<_>>(),
            vec![(240, 30), (360, 30), (1440, 30)]
        );
        assert_eq!(after.phrases[1], chart.phrases[1]);
        assert_eq!(after.phrases[0].midi, chart.phrases[0].midi);
        assert_eq!(
            (after.phrases[0].start, after.phrases[0].end),
            if second == 363 {
                (197, 467)
            } else {
                (200, 470)
            }
        );
        apply(&store, Action::Undo).unwrap();
        assert_eq!(store.load().unwrap().content.vocals.unwrap(), chart);
    }
}

#[test]
fn selection_scope_and_joined_word_refusal_preserve_unselected_vocals() {
    let (_root, store) = seeded(vec![gem(480, 0, 0)]);
    let mut chart = VocalChart {
        version: 1,
        resolution: 480,
        source: "Joined syllables".into(),
        notes: vec![vocal(243, "hel"), vocal(363, "lo")],
        phrases: vec![VocalPhrase {
            start: 200,
            end: 470,
            player: 1,
            midi: None,
        }],
    };
    chart.notes[0].lyric = Syllable::Text {
        text: "hel".into(),
        join_next: true,
    };
    apply(&store, Action::SetVocals { document: chart }).unwrap();
    let p = store.load().unwrap();
    let selection = Snapshot::capture(
        &p,
        &Selection {
            track: "Vocals".into(),
            ids: BTreeSet::new(),
            vocals: BTreeSet::from([0]),
        },
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        note_groups::repair::preview(&p, &p.timing(&store).unwrap(), &selection, policy(None))
            .unwrap_err()
            .to_string()
            .contains("whole joined word")
    );
    assert!(
        apply(
            &store,
            Action::EditNoteGroup {
                edit: Edit::Repair {
                    selection,
                    policy: policy(None)
                }
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}
