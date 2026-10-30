use chart_engine::{
    chart::{Chart, Note},
    editor::*,
    models,
};
use std::{fs, path::Path};

const CHART: &str = "[Song]\r\n{\r\n Resolution = 480\r\n Offset = 0.25\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n 960 = B 60000\r\n}\r\n[Events]\r\n{\r\n 0 = E \"section intro\"\r\n 480 = E \"lyric Original\"\r\n}\r\n[ExpertDrums]\r\n{\r\n 480 = N 2 0\r\n 480 = N 66 0\r\n 960 = N 0 0\r\n 0 = S 2 480\r\n}\r\n[ExpertSingle]\r\n{\r\n 240 = N 0 0\r\n 240 = N 5 0\r\n}\r\n";

fn fixture(root: &Path) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    let tone: Vec<_> = (0..44100 * 4)
        .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / 44100.0).sin() * 0.1)
        .collect();
    models::write_audio(&song.join("song.wav"), &tone, &tone).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Test\nartist = Original\ncustom = keep\n",
    )
    .unwrap();
    let base = Store::create(&song, None, &root.join("project")).unwrap();
    // Attach a real input-bound map with one synthetic review issue.
    let mut doc = base.load().unwrap();
    let map = chart_engine::review_regions::ReviewMap::build(
        4.0,
        Default::default(),
        doc.review.input_sha256.clone(),
        None,
        Default::default(),
        vec![chart_engine::review_regions::Issue::at(
            0.75,
            "ExpertDrums",
            chart_engine::review_regions::Reason::LowCandidateScore,
            &[("candidate_score", 0.3)],
        )],
    )
    .unwrap();
    doc.review = map;
    fs::write(
        base.root.join("project.json"),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();
    base
}
fn command(store: &Store, action: Action) -> Applied {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Test reviewer".into(),
            },
            action,
        })
        .unwrap()
}

#[test]
fn vocals_share_track_selection_and_undo_without_changing_instruments() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let instruments = store.load().unwrap().content.tracks;
    // The track is available before importing or generating any vocals.
    command(
        &store,
        Action::SelectTrack {
            track: "Vocals".into(),
        },
    );
    assert_eq!(store.load().unwrap().selected_track, "Vocals");
    let mut document: chart_engine::vocals::VocalChart =
        serde_json::from_str(include_str!("fixtures/vocals-model-free.json")).unwrap();
    for phrase in &mut document.phrases {
        phrase.start /= 4;
        phrase.end /= 4;
    }
    for note in &mut document.notes {
        note.start /= 4;
        note.end /= 4;
    }
    command(
        &store,
        Action::SetVocals {
            document: document.clone(),
        },
    );
    let loaded = store.load().unwrap();
    assert_eq!(loaded.selected_track, "Vocals");
    assert_eq!(loaded.content.tracks, instruments);
    command(&store, Action::Undo);
    assert!(store.load().unwrap().content.vocals.is_none());
    assert_eq!(store.load().unwrap().selected_track, "Vocals");
    command(&store, Action::Redo);
    assert_eq!(store.load().unwrap().content.vocals, Some(document));
    command(
        &store,
        Action::SelectTrack {
            track: "ExpertDrums".into(),
        },
    );
    assert_eq!(store.load().unwrap().selected_track, "ExpertDrums");
    assert_eq!(store.load().unwrap().content.tracks, instruments);
}

#[test]
fn erasing_final_vocal_note_is_undoable_and_removes_empty_phrase() {
    use chart_engine::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let document = VocalChart {
        version: 1,
        resolution: 480,
        source: "Test".into(),
        phrases: vec![VocalPhrase {
            start: 480,
            end: 960,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 480,
            end: 720,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "la".into(),
                join_next: false,
            },
        }],
    };
    command(
        &store,
        Action::SetVocals {
            document: document.clone(),
        },
    );
    command(
        &store,
        Action::SelectTrack {
            track: "Vocals".into(),
        },
    );
    command(&store, Action::DeleteVocalNote { index: 0 });
    let loaded = store.load().unwrap();
    assert!(loaded.content.vocals.is_none());
    assert_eq!(loaded.selected_track, "Vocals");
    command(&store, Action::Undo);
    assert_eq!(store.load().unwrap().content.vocals, Some(document));
}
fn note_id(store: &Store, track: &str, lane: u8) -> String {
    store.load().unwrap().content.tracks[track]
        .iter()
        .find(|n| n.note.lane == lane)
        .unwrap()
        .id
        .clone()
}

#[test]
fn note_edits_undo_reopen_and_export_preserve_events_other_tracks_and_originals() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), CHART);
    let id = note_id(&store, "ExpertDrums", 2);
    command(
        &store,
        Action::UpdateNote {
            note_id: id.clone(),
            tick: 720,
            lane: 3,
            sustain: 0,
            cymbal: true,
        },
    );
    let doc = store.load().unwrap();
    let text = doc.chart_text(&store).unwrap();
    let parsed = Chart::parse(text.clone()).unwrap();
    assert_eq!(
        parsed.notes("ExpertDrums").unwrap(),
        vec![
            Note {
                tick: 720,
                lane: 3,
                sustain: 0
            },
            Note {
                tick: 720,
                lane: 67,
                sustain: 0
            },
            Note {
                tick: 960,
                lane: 0,
                sustain: 0
            }
        ]
    );
    assert!(text.contains("0 = S 2 480\r\n"));
    assert!(
        text.contains(
            "[Events]\r\n{\r\n 0 = E \"section intro\"\r\n 480 = E \"lyric Original\"\r\n}"
        )
    );
    assert!(text.ends_with("[ExpertSingle]\r\n{\r\n 240 = N 0 0\r\n 240 = N 5 0\r\n}\r\n"));
    assert_eq!(
        fs::read_to_string(root.path().join("song/notes.chart")).unwrap(),
        CHART
    );
    command(&store, Action::Undo);
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), CHART);
    command(&store, Action::Redo);
    let reopened = Store::new(&store.root);
    assert_eq!(
        reopened.load().unwrap().find_note(&id).unwrap().1.note.tick,
        720
    );
    let export = root.path().join("export");
    command(
        &reopened,
        Action::Export {
            output: export.clone(),
        },
    );
    assert_eq!(
        fs::read_to_string(export.join("notes.chart")).unwrap(),
        text
    );
    assert_eq!(
        fs::read(export.join("song.wav")).unwrap(),
        fs::read(root.path().join("song/song.wav")).unwrap()
    );
    assert!(export.join("editor-review.json").exists());
    assert!(
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Human".into()
                },
                action: Action::Export { output: export }
            })
            .is_err()
    );
    command(&store, Action::DeleteNote { note_id: id });
    assert!(
        !store.load().unwrap().content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.lane == 67)
    );
}

#[test]
fn review_decisions_keep_actor_evidence_and_invalidate_for_changed_notes() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let doc = store.load().unwrap();
    let region = doc.review.regions[0].id.clone();
    command(
        &store,
        Action::SelectRegion {
            region_id: region.clone(),
        },
    );
    assert_eq!(store.load().unwrap().transport.loop_range.unwrap().end, 4.0);
    command(
        &store,
        Action::Decide {
            region_id: region.clone(),
            track: "ExpertDrums".into(),
            status: DecisionStatus::Reviewed,
            comment: "Compared source attack and kit assignment".into(),
        },
    );
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    let decision = doc.decision(&region, "ExpertDrums", &timing).unwrap();
    assert_eq!(decision.actor.kind, ActorKind::Ai);
    assert_eq!(decision.status, DecisionStatus::Reviewed);
    command(
        &store,
        Action::UpdateNote {
            note_id: note_id(&store, "ExpertDrums", 2),
            tick: 600,
            lane: 2,
            sustain: 0,
            cymbal: false,
        },
    );
    let doc = store.load().unwrap();
    assert!(doc.decision(&region, "ExpertDrums", &timing).is_none());
    assert_eq!(
        doc.content.decisions.len(),
        1,
        "old decision is retained as history"
    );
    command(&store, Action::Undo);
    let doc = store.load().unwrap();
    assert!(doc.decision(&region, "ExpertDrums", &timing).is_some());
    command(&store, Action::Undo);
    assert!(store.load().unwrap().content.decisions.is_empty());
    command(&store, Action::Redo);
    assert_eq!(store.load().unwrap().content.decisions.len(), 1);
}

#[test]
fn conflicting_or_invalid_commands_are_atomic_and_controls_follow_last_guitar_gem() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let old = store.load().unwrap().revision;
    command(&store, Action::Seek { seconds: 1.2 });
    let before = fs::read(store.root.join("project.json")).unwrap();
    let actor = Actor {
        kind: ActorKind::Ai,
        name: "Agent".into(),
    };
    assert!(
        store
            .apply(Request {
                expected_revision: old,
                actor: actor.clone(),
                action: Action::Seek { seconds: 0.0 }
            })
            .unwrap_err()
            .to_string()
            .contains("Revision conflict")
    );
    for action in [
        Action::Seek { seconds: 9.0 },
        Action::SetLoop {
            range: Some(TimeRange {
                start: 2.0,
                end: 1.0,
            }),
        },
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 960,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 1100,
            lane: 5,
            sustain: 0,
            cymbal: false,
        },
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 1100,
            lane: 1,
            sustain: 10,
            cymbal: false,
        },
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 240,
            lane: 7,
            sustain: 0,
            cymbal: false,
        },
    ] {
        assert!(
            store
                .apply(Request {
                    expected_revision: store.load().unwrap().revision,
                    actor: actor.clone(),
                    action
                })
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    }
    let id = note_id(&store, "ExpertSingle", 0);
    command(
        &store,
        Action::UpdateNote {
            note_id: id,
            tick: 600,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    assert!(
        store.load().unwrap().content.tracks["ExpertSingle"]
            .iter()
            .all(|n| n.note.tick == 600)
    );
    // A stale source snapshot cannot be exported or edited silently.
    fs::write(
        store.root.join("source/notes.chart"),
        CHART.replace("Original", "Changed"),
    )
    .unwrap();
    assert!(
        store
            .inspect()
            .unwrap_err()
            .to_string()
            .contains("Source snapshot changed")
    );
}

#[test]
fn transport_clock_loops_without_persisting_each_frame_and_serialization_is_strict() {
    let transport = Transport {
        playback_rate: 1.0,
        position: 1.0,
        playing: true,
        anchor_ms: 1000,
        loop_range: Some(TimeRange {
            start: 1.0,
            end: 2.0,
        }),
    };
    assert_eq!(transport.position_at(3500, 4.0), 1.5);
    assert_eq!(transport.position_at(500, 4.0), 1.0);
    assert!(serde_json::from_str::<Request>(r#"{"expected_revision":0,"actor":{"kind":"ai","name":"x"},"action":{"type":"seek","seconds":1,"extra":true}}"#).is_err());
}

#[test]
fn practice_rate_commands_pause_reanchor_and_reject_invalid_values_atomically() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    command(&store, Action::Seek { seconds: 1.5 });
    command(&store, Action::SetPlaybackRate { rate: 0.75 });
    let doc = store.load().unwrap();
    assert_eq!(doc.transport.playback_rate, 0.75);
    assert!(!doc.transport.playing);
    assert_eq!(doc.transport.position, 1.5);
    let previous = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: doc.revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Practice test".into()
                },
                action: Action::SetPlaybackRate { rate: 0.0 }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), previous);
    assert_eq!(doc.chart_text(&store).unwrap(), CHART);
}
