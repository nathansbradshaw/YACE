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
fn competing_command_cannot_save_between_revision_check_and_commit() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let revision = store.load().unwrap().revision;
    let request = |action| Request {
        expected_revision: revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Concurrent editor".into(),
        },
        action,
    };
    let mut rejected = false;
    store
        .apply_guarded(
            request(Action::SelectTrack {
                track: "ExpertSingle".into(),
            }),
            |_| {
                let error = store
                    .clone()
                    .apply(request(Action::Seek { seconds: 2.0 }))
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("Another editor command is saving")
                );
                rejected = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(rejected);
    let saved = store.load().unwrap();
    assert_eq!(saved.revision, revision + 1);
    assert_eq!(saved.selected_track, "ExpertSingle");
    assert_eq!(saved.audit.len(), 1);
    // The command guard is released after success, allowing a fresh revision.
    command(&store, Action::Seek { seconds: 2.0 });
    assert_eq!(store.load().unwrap().revision, revision + 2);
}

#[test]
fn cached_source_rejects_same_length_tampering_and_recovers_after_restoration() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let original = store.load().unwrap();
    let expected = original.chart_text(&store).unwrap();
    let source = store.root.join("source/notes.chart");
    let modified = fs::metadata(&source).unwrap().modified().unwrap();
    let replacement = CHART.replace("Original", "Tampered");
    assert_eq!(replacement.len(), CHART.len());
    fs::write(&source, replacement).unwrap();
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    // A clone shares cached source data; it must still check actual current bytes.
    assert!(
        original
            .chart_text(&store.clone())
            .unwrap_err()
            .to_string()
            .contains("Source snapshot changed")
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: original.revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Cache test".into()
                },
                action: Action::Seek { seconds: 1.0 },
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    fs::write(&source, CHART).unwrap();
    assert_eq!(original.chart_text(&store.clone()).unwrap(), expected);
    command(&store, Action::Seek { seconds: 1.0 });
}

#[test]
fn session_commands_still_reject_corrupt_saved_content() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let mut project = store.load().unwrap();
    // A warmed source cache must never act as a cache of validated project.json.
    let duplicate = project.content.tracks["ExpertSingle"][0].clone();
    project
        .content
        .tracks
        .get_mut("ExpertSingle")
        .unwrap()
        .push(duplicate);
    let bytes = serde_json::to_vec_pretty(&project).unwrap();
    fs::write(store.root.join("project.json"), &bytes).unwrap();
    for action in [
        Action::Seek { seconds: 1.0 },
        Action::Playback { playing: true },
        Action::SelectTrack {
            track: "ExpertSingle".into(),
        },
    ] {
        assert!(
            store
                .apply(Request {
                    expected_revision: project.revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: "Validation test".into()
                    },
                    action,
                })
                .is_err()
        );
    }
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), bytes);
}

#[test]
fn pause_saves_the_audio_cursor_once_without_drifting_or_changing_notes() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    command(&store, Action::Playback { playing: true });
    let playing = store.load().unwrap();
    let cursor = 1.2345;
    command(&store, Action::Pause { seconds: cursor });
    let paused = store.load().unwrap();
    assert_eq!(paused.revision, playing.revision + 1);
    assert!(!paused.transport.playing);
    assert_eq!(paused.transport.position, cursor);
    // Saving or displaying later cannot advance the frozen audio cursor.
    assert_eq!(
        paused.transport.position_at(u64::MAX, paused.duration),
        cursor
    );
    assert_eq!(paused.content.tracks, playing.content.tracks);
    assert_eq!(paused.undo, playing.undo);
    assert_eq!(paused.redo, playing.redo);
    assert!(
        matches!(paused.audit.last().unwrap().action, Action::Pause { seconds } if seconds == cursor)
    );
}

#[test]
fn invalid_pause_positions_do_not_change_the_saved_transport() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    command(&store, Action::Playback { playing: true });
    let before = store.load().unwrap();
    for seconds in [-0.1, before.duration + 0.1, f64::NAN, f64::INFINITY] {
        assert!(
            store
                .apply(Request {
                    expected_revision: before.revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: "Pause test".into()
                    },
                    action: Action::Pause { seconds },
                })
                .is_err()
        );
        assert_eq!(store.load().unwrap().revision, before.revision);
    }
}

#[cfg(feature = "editor")]
mod pause_ui {
    use super::*;
    use chart_engine::editor::{Action, Store};
    use chart_engine::studio::state::{Studio, use_studio};
    use dioxus::prelude::*;
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    type Slot = Rc<RefCell<Option<Studio>>>;
    fn paused_window() -> Element {
        let mut studio = use_studio();
        let (store, stale, slot) = use_context::<(Store, Project, Slot)>();
        use_hook(move || {
            studio.store.set(Some(store));
            studio.doc.set(Some(Arc::new(stale)));
            studio.playhead.set(1.5);
            studio.dispatch(Action::Playback { playing: false });
            *slot.borrow_mut() = Some(studio);
        });
        rsx! { div {} }
    }

    #[tokio::test]
    async fn a_save_conflict_does_not_resume_the_local_transport() {
        let temp = tempfile::tempdir().unwrap();
        let store = fixture(temp.path());
        let mut stale = store.load().unwrap();
        stale.transport.playing = true;
        command(&store, Action::Playback { playing: true });
        let revision = store.load().unwrap().revision;
        let slot = Rc::new(RefCell::new(None::<Studio>));
        let mut dom = VirtualDom::new(paused_window);
        dom.provide_root_context((store.clone(), stale, slot.clone()));
        dom.rebuild_to_vec();
        let studio = slot.borrow().unwrap();
        assert!(!studio.snapshot().unwrap().transport.playing);
        assert_eq!(studio.snapshot().unwrap().transport.position, 1.5);
        assert_eq!((studio.playhead)(), 1.5);
        assert!((studio.error)().unwrap().contains("Revision conflict"));
        assert_eq!(store.load().unwrap().revision, revision);
    }

    fn ended_window() -> Element {
        let mut studio = use_studio();
        let (store, slot) = use_context::<(Store, Slot)>();
        use_hook(move || {
            let mut project = store.load().unwrap();
            project.transport.playing = true;
            studio.playhead.set(project.duration + 0.001);
            studio.store.set(Some(store));
            studio.doc.set(Some(Arc::new(project)));
            studio.dispatch(Action::Playback { playing: false });
            *slot.borrow_mut() = Some(studio);
        });
        rsx! { div {} }
    }

    #[tokio::test]
    async fn an_implicit_pause_clamps_output_rounding_at_the_end_of_the_recording() {
        let temp = tempfile::tempdir().unwrap();
        let store = fixture(temp.path());
        let slot = Rc::new(RefCell::new(None::<Studio>));
        let mut dom = VirtualDom::new(ended_window);
        dom.provide_root_context((store.clone(), slot.clone()));
        dom.rebuild_to_vec();
        let studio = slot.borrow().unwrap();
        assert!((studio.error)().is_none());
        let saved = store.load().unwrap();
        assert!(!saved.transport.playing);
        assert_eq!(saved.transport.position, saved.duration);
    }
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
            midi: None,
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
    assert!(chart_engine::song_layout::path(&export, "editor-review.json").exists());
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
fn play_restarts_a_completed_recording_without_changing_rate_or_chart() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = store.load().unwrap();
    for rate in [0.25, 0.83, 2.] {
        command(&store, Action::SetPlaybackRate { rate });
        command(&store, Action::Pause { seconds: before.duration });
        command(&store, Action::Playback { playing: true });
        let replay = store.load().unwrap();
        assert!(replay.transport.playing);
        assert_eq!(replay.transport.position, 0.);
        assert_eq!(replay.transport.playback_rate, rate);
        assert_eq!(
            serde_json::to_value(&replay.content).unwrap(),
            serde_json::to_value(&before.content).unwrap()
        );
        assert_eq!(replay.audio_sha256, before.audio_sha256);
    }
}

#[test]
fn practice_rate_commands_preserve_play_state_and_reject_invalid_values_atomically() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    command(&store, Action::Seek { seconds: 1.5 });
    command(&store, Action::SetPlaybackRate { rate: 0.75 });
    let doc = store.load().unwrap();
    assert_eq!(doc.transport.playback_rate, 0.75);
    assert!(!doc.transport.playing);
    assert_eq!(doc.transport.position, 1.5);
    command(&store, Action::Playback { playing: true });
    let playing = store.load().unwrap();
    command(&store, Action::SetPlaybackRate { rate: 1.25 });
    let doc = store.load().unwrap();
    assert_eq!(doc.transport.playback_rate, 1.25);
    assert!(doc.transport.playing);
    assert!(doc.transport.position >= playing.transport.position);
    assert!(
        doc.transport.position
            <= playing
                .transport
                .position_at(doc.transport.anchor_ms, doc.duration)
    );
    for rate in [0.25, 0.5, 0.83, 1., 1.5, 2.] {
        command(&store, Action::SetPlaybackRate { rate });
        let changed = store.load().unwrap();
        assert!(changed.transport.playing);
        assert_eq!(changed.transport.playback_rate, rate);
    }
    command(&store, Action::Pause { seconds: 2. });
    for rate in [0.25, 0.83, 2.] {
        command(&store, Action::SetPlaybackRate { rate });
        let changed = store.load().unwrap();
        assert!(!changed.transport.playing);
        assert_eq!(changed.transport.position, 2.);
    }
    let doc = store.load().unwrap();
    let previous = fs::read(store.root.join("project.json")).unwrap();
    for rate in [0., 0.24, 2.01, f64::NAN, f64::INFINITY] {
        let error = store
            .apply(Request {
                expected_revision: doc.revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Practice test".into(),
                },
                action: Action::SetPlaybackRate { rate },
            })
            .unwrap_err();
        assert!(error.to_string().contains("Practice rate"), "{error:#}");
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), previous);
    }
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), previous);
    assert_eq!(doc.chart_text(&store).unwrap(), CHART);
}
