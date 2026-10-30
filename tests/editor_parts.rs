use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    parts,
    timing::{Tempo, TempoMap},
};
use std::{fs, path::Path};
mod fixture {
    include!("support/assistant_project.rs");
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Part test".into(),
            },
            action,
        })
        .unwrap();
}

#[test]
fn every_part_can_start_blank_and_survives_undo_redo_reopen_and_export() {
    for (_, suffix, _) in parts::PARTS {
        let root = tempfile::tempdir().unwrap();
        let store = fixture::fixture(root.path());
        let before = store.load().unwrap();
        let original_source = fs::read(store.root.join("source/notes.chart")).unwrap();
        let track = if suffix == "Vocals" {
            "Vocals".to_owned()
        } else {
            format!("Hard{suffix}")
        };
        apply(
            &store,
            Action::CreateTrack {
                track: track.clone(),
            },
        );
        let added = store.load().unwrap();
        assert_eq!(added.selected_track, track);
        assert!(added.content.track_origins.contains_key(&track));
        assert_eq!(
            serde_json::to_value(&added.content.tracks["ExpertSingle"]).unwrap(),
            serde_json::to_value(&before.content.tracks["ExpertSingle"]).unwrap()
        );
        assert_eq!(
            added.chart_text(&store).unwrap()[..before.chart_text(&store).unwrap().len()],
            before.chart_text(&store).unwrap()
        );
        let revision = added.revision;
        assert!(
            store
                .apply(Request {
                    expected_revision: revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: "Part test".into()
                    },
                    action: Action::CreateTrack {
                        track: track.clone()
                    }
                })
                .is_err()
        );
        assert_eq!(store.load().unwrap().revision, revision);
        apply(&store, Action::Undo);
        assert!(
            !store
                .load()
                .unwrap()
                .content
                .track_origins
                .contains_key(&track)
        );
        apply(&store, Action::Redo);
        assert!(
            store
                .load()
                .unwrap()
                .content
                .track_origins
                .contains_key(&track)
        );
        if suffix != "Vocals" {
            apply(
                &store,
                Action::AddNote {
                    track: track.clone(),
                    tick: 720,
                    lane: if suffix == "ProKeys" { 60 } else { 0 },
                    sustain: 0,
                    cymbal: false,
                },
            );
        }
        let export = root.path().join("export");
        apply(
            &store,
            Action::Export {
                output: export.clone(),
            },
        );
        assert!(
            export
                .join(if suffix == "ProKeys" {
                    "notes.mid"
                } else {
                    "notes.chart"
                })
                .exists()
        );
        assert_eq!(
            fs::read(store.root.join("source/notes.chart")).unwrap(),
            original_source
        );
    }
}

#[test]
fn unknown_tracks_and_unregistered_sections_still_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: 0,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Part test".into()
                },
                action: Action::CreateTrack {
                    track: "ExpertAnything".into()
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    let mut doc = store.load().unwrap();
    doc.content.tracks.insert("ExpertKeyboard".into(), vec![]);
    let chart = chart_engine::chart::Chart::parse(
        fs::read_to_string(store.root.join("source/notes.chart")).unwrap(),
    )
    .unwrap();
    assert!(chart_engine::editor_validation::validate(&doc, &chart).is_err());
}

// These model-free drafts test the import boundary, not transcription quality.
fn draft(store: &Store, job: &Path, track: &str) {
    fs::create_dir(job).unwrap();
    let target = parts::Target::capture(store, &store.load().unwrap(), track).unwrap();
    fs::create_dir(job.join("project")).unwrap();
    let generated = fixture::fixture(&job.join("project"));
    if track == "Vocals" {
        use chart_engine::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
        apply(
            &generated,
            Action::SetVocals {
                document: VocalChart {
                    version: 1,
                    resolution: 480,
                    source: "Model-free import fixture".into(),
                    phrases: vec![VocalPhrase {
                        start: 480,
                        end: 1440,
                        player: 1,
                    }],
                    notes: vec![VocalNote {
                        start: 480,
                        end: 960,
                        tone: VocalTone::Pitched { midi: 60 },
                        lyric: Syllable::Missing,
                    }],
                },
            },
        );
    } else {
        if !generated.load().unwrap().content.tracks.contains_key(track) {
            apply(
                &generated,
                Action::CreateTrack {
                    track: track.into(),
                },
            );
        }
        apply(
            &generated,
            Action::AddNote {
                track: track.into(),
                tick: 960,
                lane: if track.ends_with("ProKeys") { 60 } else { 2 },
                sustain: 0,
                cymbal: track.ends_with("Drums"),
            },
        );
    }
    let request = serde_json::json!({"source":{"type":"audio","path":store.audio_path(&store.load().unwrap()).unwrap()},"bpm":120.,"title":"Draft","artist":"Fixture","instruments":[parts::instrument(track).unwrap()],"model_dir":"unused","fetch_models":false});
    chart_engine::generation::write_json(&job.join("request.json"), &request).unwrap();
    chart_engine::generation::write_json(&job.join("editor-target.json"), &target).unwrap();
    chart_engine::generation::write_json(
        &job.join("job.json"),
        &chart_engine::generation::Status {
            version: 1,
            state: chart_engine::generation::State::Complete,
            stage: "complete".into(),
            detail: "Model-free import fixture".into(),
            request_sha256: chart_engine::models::hash_file(&job.join("request.json")).unwrap(),
            error: None,
        },
    )
    .unwrap();
}

#[test]
fn generated_parts_preserve_unrelated_edits_and_have_one_undo_step() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let job = root.path().join("draft");
    draft(&store, &job, "ExpertKeyboard");
    apply(
        &store,
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 1440,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    let before = store.load().unwrap();
    let drums = serde_json::to_value(&before.content.tracks["ExpertDrums"]).unwrap();
    apply(&store, Action::ApplyGeneratedPart { job });
    let imported = store.load().unwrap();
    assert_eq!(imported.content.tracks["ExpertKeyboard"].len(), 1);
    assert_eq!(
        serde_json::to_value(&imported.content.tracks["ExpertDrums"]).unwrap(),
        drums
    );
    assert_eq!(imported.undo.len(), before.undo.len() + 1);
    assert!(matches!(
        imported.content.track_origins["ExpertKeyboard"],
        parts::Origin::Generated { .. }
    ));
    apply(&store, Action::Undo);
    assert!(
        !store
            .load()
            .unwrap()
            .content
            .tracks
            .contains_key("ExpertKeyboard")
    );
    assert_eq!(
        serde_json::to_value(&store.load().unwrap().content.tracks["ExpertDrums"]).unwrap(),
        drums
    );
    apply(&store, Action::Redo);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertKeyboard"].len(),
        1
    );
}

#[test]
fn changing_the_target_or_recording_rejects_the_draft_atomically() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let job = root.path().join("draft");
    draft(&store, &job, "ExpertDrums");
    apply(
        &store,
        Action::AddNote {
            track: "ExpertDrums".into(),
            tick: 1440,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Part test".into()
                },
                action: Action::ApplyGeneratedPart { job: job.clone() }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(&store, Action::Undo);
    fs::write(
        store.audio_path(&store.load().unwrap()).unwrap(),
        b"changed recording",
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Part test".into()
                },
                action: Action::ApplyGeneratedPart { job }
            })
            .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}

#[test]
fn part_generation_keeps_tempo_changes_offset_and_resolution() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let mut project = store.load().unwrap();
    let source = store.root.join("source/notes.chart");
    let chart = fs::read_to_string(&source)
        .unwrap()
        .replace("Resolution = 480", "Resolution = 960\n Offset = 0.25")
        .replace("0 = B 120000", "0 = B 123000\n 960 = B 90000");
    fs::write(&source, chart).unwrap();
    project.source_chart_sha256 = chart_engine::models::hash_file(&source).unwrap();
    project
        .review
        .input_sha256
        .insert("chart".into(), project.source_chart_sha256.clone());
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    let mut target =
        parts::Target::capture(&store, &store.load().unwrap(), "ExpertKeyboard").unwrap();
    let original = target.timing.clone();
    assert_eq!(original.resolution, 960);
    assert_eq!(original.offset, 0.25);
    assert_eq!(original.tempos.len(), 2);
    let job = root.path().join("draft");
    fs::create_dir(&job).unwrap();
    chart_engine::generation::write_json(&job.join("editor-target.json"), &target).unwrap();
    assert_eq!(parts::timing(&job, 99.).unwrap(), original);
    target.timing = TempoMap::new(
        192,
        0.25,
        vec![
            Tempo { tick: 0, bpm: 123. },
            Tempo {
                tick: 192,
                bpm: 90.,
            },
        ],
    )
    .unwrap();
    chart_engine::generation::write_json(&job.join("editor-target.json"), &target).unwrap();
    assert!(parts::timing(&job, 120.).is_err());
}

#[test]
fn every_generated_part_uses_the_same_guarded_import_path() {
    for track in [
        "ExpertSingle",
        "ExpertDoubleBass",
        "ExpertDrums",
        "ExpertKeyboard",
        "ExpertProKeys",
        "Vocals",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = fixture::fixture(root.path());
        let before = store.load().unwrap();
        let job = root.path().join("draft");
        draft(&store, &job, track);
        apply(&store, Action::ApplyGeneratedPart { job });
        let imported = store.load().unwrap();
        assert_eq!(imported.selected_track, track);
        if track == "Vocals" {
            assert!(matches!(
                imported.content.vocals.as_ref().unwrap().notes[0].lyric,
                chart_engine::vocals::Syllable::Missing
            ));
        } else {
            assert!(
                imported.content.tracks[track]
                    .iter()
                    .any(|item| item.note.tick == 960)
            );
        }
        apply(&store, Action::Undo);
        assert_eq!(
            serde_json::to_value(store.load().unwrap().content).unwrap(),
            serde_json::to_value(before.content).unwrap()
        );
    }
}

#[test]
fn a_new_expert_part_can_create_linked_lower_difficulties() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    apply(
        &store,
        Action::CreateTrack {
            track: "ExpertDoubleBass".into(),
        },
    );
    apply(
        &store,
        Action::AddNote {
            track: "ExpertDoubleBass".into(),
            tick: 960,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    apply(
        &store,
        Action::EnableDifficultyLink {
            track: "HardDoubleBass".into(),
            enabled: true,
        },
    );
    assert!(
        store
            .load()
            .unwrap()
            .content
            .tracks
            .contains_key("HardDoubleBass")
    );
    assert!(
        store
            .load()
            .unwrap()
            .chart_text(&store)
            .unwrap()
            .contains("[HardDoubleBass]")
    );
}
