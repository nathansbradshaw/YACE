use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    storage as fs,
};
use std::path::{Path, PathBuf};

const CHART: &str = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n0 = TS 5 3\n960 = B 150000\n}\n[Events]\n{\n4800 = E \"section later\"\n}\n[ExpertSingle]\n{\n480 = N 0 480\n1440 = N 1 240\n}\n[ExpertKeyboard]\n{\n480 = N 4 240\n}\n";
fn source(root: &Path) -> PathBuf {
    let path = root.join("song");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("notes.chart"), CHART).unwrap();
    fs::write(
        path.join("song.ini"),
        "[song]\nname = Chart only\nartist = Fixture\ncustom = keep\n",
    )
    .unwrap();
    path
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Chart-only fixture".into(),
            },
            action,
        })
        .unwrap();
}
fn audio_files(path: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(audio_files(&path));
        } else if path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            ["wav", "ogg", "flac", "mp3", "opus", "aiff", "aac", "m4a"].contains(&s)
        }) {
            result.push(path);
        }
    }
    result
}

#[test]
fn chart_only_edits_export_and_recovery_keep_history_without_fake_audio() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let doc = store.load().unwrap();
    assert!(!doc.has_recording());
    assert_eq!(doc.duration, 60.);
    assert!(doc.audio_sha256.is_empty());
    assert!(store.audio_path(&doc).is_err());
    assert!(!doc.review.input_sha256.contains_key("original_audio"));
    assert!(doc.review.regions.is_empty());
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 2400,
            lane: 3,
            sustain: 240,
            cymbal: false,
        },
    );
    let edited = store.load().unwrap().content.tracks["ExpertSingle"].len();
    apply(&store, Action::Undo);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        edited - 1
    );
    apply(&store, Action::Redo);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        edited
    );
    let export = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: export.clone(),
        },
    );
    assert!(audio_files(&export).is_empty());
    for sidecar in ["yace-audio.json", "chart-engine-melody-source.json"] {
        assert!(!fs::exists(crate::song_layout::resolve(&export, sidecar)));
    }
    let reopened = Store::create(&export, None, &root.path().join("reopened")).unwrap();
    assert!(!reopened.load().unwrap().has_recording());
    assert_eq!(
        reopened.load().unwrap().content.tracks["ExpertSingle"].len(),
        edited
    );
    #[cfg(any(feature = "editor", feature = "browser"))]
    {
        let zip = root.path().join("backup.zip");
        crate::project_files::backup(&store, &zip).unwrap();
        let restored = root.path().join("restored");
        let restored = crate::project_files::import(&zip, &restored).unwrap();
        apply(&restored, Action::Undo);
        apply(&restored, Action::Redo);
        assert_eq!(
            restored.load().unwrap().content.tracks["ExpertSingle"].len(),
            edited
        );
        assert!(audio_files(&restored.root).is_empty());
    }
    assert_eq!(fs::read_to_string(song.join("notes.chart")).unwrap(), CHART);
    assert!(!fs::exists(song.join(".yace")));
}

#[test]
fn midi_only_import_retains_original_bytes_on_unchanged_export() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    fs::remove_file(song.join("notes.chart")).unwrap();
    let midi = crate::midi::new_midi(
        &crate::timing::TempoMap::constant(120.).unwrap(),
        &BTreeMap::from([(
            "ExpertSingle".into(),
            vec![crate::chart::Note {
                tick: 480,
                lane: 2,
                sustain: 480,
            }],
        )]),
    )
    .unwrap();
    fs::write(song.join("notes.mid"), &midi).unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    assert!(!store.load().unwrap().has_recording());
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    assert_eq!(fs::read(output.join("notes.mid")).unwrap(), midi);
    assert!(audio_files(&output).is_empty());
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(!reopened.load().unwrap().has_recording());
}

#[test]
fn source_bound_missing_recording_is_refused_and_long_chart_extent_is_not_truncated() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    crate::song_layout::prepare(&song).unwrap();
    let manifest = crate::song_layout::path(&song, "chart-engine-isolation.json");
    fs::write(&manifest, b"{}").unwrap();
    let output = root.path().join("refused");
    let error = Store::create(&song, None, &output).err().unwrap();
    assert!(error.to_string().contains("recording is missing"));
    assert!(!fs::exists(output));
    fs::remove_file(manifest).unwrap();
    fs::write(
        song.join("notes.chart"),
        CHART.replace("1440 = N 1 240", "144000 = N 1 960"),
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("long")).unwrap();
    let doc = store.load().unwrap();
    let end = doc.timing(&store).unwrap().seconds(144960);
    assert!(doc.duration >= end + 8.);
    assert!(audio_files(&store.root).is_empty());
}

#[test]
fn blank_duration_grows_and_explicit_duration_is_undoable_and_survives_export() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    let (ini, chart) = blank_song("Blank", "Unknown artist", 120., 2.).unwrap();
    fs::write(song.join("song.ini"), ini).unwrap();
    fs::write(song.join("notes.chart"), chart).unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    assert_eq!(store.load().unwrap().duration, 2.);
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 4800,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    assert_eq!(store.load().unwrap().duration, 13.);
    apply(&store, Action::Undo);
    assert_eq!(store.load().unwrap().duration, 2.);
    apply(&store, Action::Redo);
    assert_eq!(store.load().unwrap().duration, 13.);
    apply(&store, Action::SetTimelineDuration { duration: 7. });
    assert_eq!(store.load().unwrap().duration, 7.);
    let revision = store.load().unwrap().revision;
    assert!(
        store
            .apply(Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "fixture".into()
                },
                action: Action::SetTimelineDuration { duration: 1. }
            })
            .is_err()
    );
    assert_eq!(store.load().unwrap().revision, revision);
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(reopened.load().unwrap().duration, 7.);
    assert!(audio_files(&output).is_empty());
}

#[test]
fn recording_attachment_undo_redo_and_backup_retain_notes_and_real_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 2400,
            lane: 3,
            sustain: 240,
            cymbal: false,
        },
    );
    let count = store.load().unwrap().content.tracks["ExpertSingle"].len();
    let audio = root.path().join("chosen.wav");
    let samples = vec![0.05; 441000];
    crate::models::write_audio(&audio, &samples, &samples).unwrap();
    let prepared = prepare_recording(&audio, &store.root).unwrap();
    let hash = prepared.recording.sha256.clone();
    fs::remove_file(&audio).unwrap();
    apply(&store, Action::AttachRecording { prepared });
    let doc = store.load().unwrap();
    assert!(doc.has_recording());
    assert_eq!(doc.duration, 10.);
    assert_eq!(
        crate::models::hash_file(&store.audio_path(&doc).unwrap()).unwrap(),
        hash
    );
    apply(&store, Action::Undo);
    assert!(!store.load().unwrap().has_recording());
    let no_audio = root.path().join("unattached");
    apply(
        &store,
        Action::Export {
            output: no_audio.clone(),
        },
    );
    assert!(!fs::is_file(no_audio.join("song.wav")));
    apply(&store, Action::Undo);
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        count - 1
    );
    apply(&store, Action::Redo);
    apply(&store, Action::Redo);
    assert!(store.load().unwrap().has_recording());
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        count
    );
    let output = root.path().join("attached");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    assert_eq!(
        crate::models::hash_file(&output.join("song.wav")).unwrap(),
        hash
    );
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(reopened.load().unwrap().has_recording());
    #[cfg(any(feature = "editor", feature = "browser"))]
    {
        let backup = root.path().join("backup.zip");
        crate::project_files::backup(&store, &backup).unwrap();
        let restored =
            crate::project_files::import(&backup, &root.path().join("restored")).unwrap();
        apply(&restored, Action::Undo);
        assert!(!restored.load().unwrap().has_recording());
        apply(&restored, Action::Redo);
        assert!(restored.load().unwrap().has_recording());
        assert_eq!(
            restored.load().unwrap().content.tracks["ExpertSingle"].len(),
            count
        );
    }
}

#[test]
fn too_short_or_changed_recording_never_changes_the_project_revision() {
    let root = tempfile::tempdir().unwrap();
    let song = source(root.path());
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let audio = root.path().join("short.wav");
    crate::models::write_audio(&audio, &vec![0.; 44100], &vec![0.; 44100]).unwrap();
    let prepared = prepare_recording(&audio, &store.root).unwrap();
    let revision = store.load().unwrap().revision;
    assert!(
        store
            .apply(Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "fixture".into()
                },
                action: Action::AttachRecording {
                    prepared: prepared.clone()
                }
            })
            .is_err()
    );
    assert_eq!(store.load().unwrap().revision, revision);
    assert!(!store.load().unwrap().has_recording());
    fs::write(
        store.root.join("source").join(&prepared.recording.filename),
        b"damaged",
    )
    .unwrap();
    assert!(verify_recording(&store.root, &prepared).is_err());
    assert_eq!(store.load().unwrap().revision, revision);
}
