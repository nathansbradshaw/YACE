//! Exercise the real browser working set natively, without browser globals.
#![allow(dead_code)]
use chart_engine::{editor, project_portability, project_recording};
#[path = "../src/archive_codec.rs"]
mod archive_codec;
#[path = "../src/compact_backup.rs"]
mod compact_backup;
use std::path::{Path, PathBuf};
mod files {
    use std::path::{Path, PathBuf};
    pub fn clear_downloads(_: &Path) {}
    pub fn unique_root() -> PathBuf {
        PathBuf::from("/test-id")
    }
}
mod part_analysis {
    pub fn forget_root(_: &std::path::Path) {}
}
mod project_folder {
    // Native working-set tests have no browser folder handles to release.
    pub fn forget(_: &std::path::Path) {}
}
#[path = "support/assistant_project.rs"]
mod fixture;
#[path = "../web/src/project_integrity.rs"]
mod project_integrity;
#[path = "../web/src/project_recovery.rs"]
mod project_recovery;
#[path = "../web/src/storage.rs"]
mod storage;
fn document() -> editor::Project {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture::fixture(temp.path());
    serde_json::from_slice(&std::fs::read(store.root.join("project.json")).unwrap()).unwrap()
}
#[test]
fn retirement_waits_for_all_inflight_project_readers() {
    let root = Path::new("/pending-project-readers");
    storage::write(root.join("source/audio.wav"), b"recording").unwrap();
    let older = storage::hold_project(root);
    let newer = storage::hold_project(root);
    drop(older);
    storage::clear(root);
    assert_eq!(
        storage::read(root.join("source/audio.wav")).unwrap(),
        b"recording"
    );
    drop(newer);
    assert!(!storage::exists(root));
}
#[test]
fn successful_open_takes_ownership_before_its_reader_is_released() {
    let root = Path::new("/pending-project-transfer/project");
    storage::write(root.join("source/audio.wav"), b"recording").unwrap();
    let reader = storage::hold_project(root);
    storage::clear(root.parent().unwrap());
    storage::activate_project(Some(root));
    drop(reader);
    assert_eq!(
        storage::read(root.join("source/audio.wav")).unwrap(),
        b"recording"
    );
    storage::activate_project(None);
    assert!(!storage::exists(root));
}
#[test]
fn active_generated_project_survives_cleanup_until_another_project_owns_the_editor() {
    let root = Path::new("/generated-open/project");
    let next = Path::new("/other-open/project");
    storage::save_project(root, &document());
    storage::write(root.join("source/song.ogg"), b"recording").unwrap();
    storage::activate_project(Some(root));
    storage::clear(root);
    storage::clear(root.parent().unwrap());
    assert!(storage::is_file(root.join("project.json")));
    assert_eq!(
        storage::read(root.join("source/song.ogg")).unwrap(),
        b"recording"
    );
    // Ordinary file replacement still works inside the protected project.
    storage::write(root.join("transport.tmp"), b"new").unwrap();
    storage::rename(root.join("transport.tmp"), root.join("transport.json")).unwrap();
    assert_eq!(storage::read(root.join("transport.json")).unwrap(), b"new");
    storage::save_project(next, &document());
    storage::activate_project(Some(next));
    storage::clear(root);
    assert!(!storage::exists(root));
    assert!(storage::is_file(next.join("project.json")));
    storage::activate_project(None);
    storage::clear(next);
}
#[test]
fn incomplete_snapshot_cannot_be_acknowledged_as_a_saved_project() {
    let root = Path::new("/incomplete-snapshot");
    let mut doc = document();
    doc.undo.push("a".repeat(64));
    storage::save_project(root, &doc);
    let validate = || {
        project_integrity::validate(
            root,
            doc.revision,
            &storage::list(root).unwrap(),
            storage::cached_project(root),
        )
    };
    assert!(
        validate()
            .unwrap_err()
            .to_string()
            .contains("source/notes.chart")
    );
    storage::write(root.join("source/notes.chart"), b"source").unwrap();
    assert!(
        validate()
            .unwrap_err()
            .to_string()
            .contains("source/song.ogg")
    );
    storage::write(root.join("source/song.ogg"), b"recording").unwrap();
    assert!(validate().unwrap_err().to_string().contains("checkpoints/"));
    storage::write(
        root.join(format!("checkpoints/{}.json", "a".repeat(64))),
        b"checkpoint",
    )
    .unwrap();
    validate().unwrap();
    let files = storage::list(root).unwrap();
    project_integrity::validate(root, doc.revision, &files, None).unwrap();
    assert!(project_integrity::validate(root, doc.revision + 1, &files, None).is_err());
    storage::clear(root);
    assert!(
        project_integrity::validate(root, doc.revision, &storage::list(root).unwrap(), None)
            .is_err()
    );
}
#[test]
fn cached_only_project_materializes_for_reads_archives_and_pending_saves() {
    let root = Path::new("/cache-test");
    storage::clear(root);
    let doc = document();
    storage::remember_project(root, &doc);
    assert!(storage::exists(root));
    assert!(storage::exists(root.join("project.json")));
    let files = storage::list(root).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(storage::pending(root).unwrap().len(), 1);
    storage::acknowledge(storage::pending(root).unwrap());
    let decoded: editor::Project =
        serde_json::from_slice(&storage::read(root.join("project.json")).unwrap()).unwrap();
    assert_eq!(decoded.title, doc.title);
    let mut edited = doc.clone();
    edited.revision += 1;
    edited.title = "Edited title".into();
    storage::save_project(root, &edited);
    let pending = storage::pending(root).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        serde_json::from_slice::<editor::Project>(&pending[0].1)
            .unwrap()
            .revision,
        edited.revision
    );
    storage::acknowledge(pending);
    assert!(storage::pending(root).unwrap().is_empty());
    // A save completing after the working set was lost must not acknowledge
    // the newly recovered document (or borrow DIRTY while materializing it).
    storage::save_project(root, &edited);
    let stale = storage::pending(root).unwrap();
    storage::clear(root);
    edited.revision += 1;
    storage::remember_project(root, &edited);
    storage::acknowledge(stale);
    assert_eq!(storage::pending(root).unwrap().len(), 1);
    storage::clear(root);
}
#[test]
fn missing_file_error_identifies_the_asset() {
    let path = Path::new("/missing/source/song.ogg");
    let error = storage::shared(path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains(&path.display().to_string()));
}
#[test]
fn recovery_keeps_live_edits_available_files_and_history_without_loading_store() {
    let root = Path::new("/damaged-project");
    let output = Path::new("/rescued-project");
    storage::clear(root);
    storage::clear(output);
    let mut doc = document();
    doc.revision = 7;
    doc.title = "Unsaved title".into();
    doc.content.vocals = Some(chart_engine::vocals::VocalChart {
        version: 1,
        resolution: 480,
        source: "Authored recovery test".into(),
        phrases: vec![chart_engine::vocals::VocalPhrase {
            start: 0,
            end: 1920,
            player: 1,
        }],
        notes: vec![chart_engine::vocals::VocalNote {
            start: 480,
            end: 960,
            tone: chart_engine::vocals::VocalTone::Pitched { midi: 60 },
            lyric: chart_engine::vocals::Syllable::Text {
                text: "todavía".into(),
                join_next: false,
            },
            midi: None,
        }],
    });
    doc.undo.push("a".repeat(64));
    storage::write(root.join("source/extras.bin"), [0, 255, 21]).unwrap();
    let report = project_recovery::prepare(root, &doc, output).unwrap();
    assert!(report.source_error.is_some());
    assert_eq!(
        report.missing_checkpoints,
        vec![format!("checkpoints/{}.json", "a".repeat(64))]
    );
    assert!(!storage::is_file(root.join("project.json")));
    assert!(storage::cached_project(root).is_none());
    let recovered: editor::Project =
        serde_json::from_slice(&storage::read(output.join("project.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&recovered).unwrap(),
        serde_json::to_value(&doc).unwrap()
    );
    assert_eq!(
        storage::read(output.join("source/extras.bin")).unwrap(),
        [0, 255, 21]
    );
    let files = storage::list(output).unwrap();
    let names = files
        .iter()
        .map(|(p, _)| p.strip_prefix(output).unwrap().to_str().unwrap())
        .collect::<Vec<_>>();
    let zip = archive_codec::encode(
        names
            .iter()
            .zip(&files)
            .map(|(name, (_, bytes))| (*name, bytes.as_slice())),
    )
    .unwrap();
    let decoded = archive_codec::decode_shared(&zip).unwrap();
    assert!(decoded.iter().any(|(name, _)| name == "project.json"));
    assert!(
        decoded
            .iter()
            .any(|(name, _)| name == "yace-recovery/report.json")
    );
    storage::clear(root);
    storage::clear(output);
}
#[test]
fn recovered_missing_manifest_reopens_with_its_source_and_exact_undo_history() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture::fixture(temp.path());
    // Create real content-addressed history with the normal native store.
    store
        .apply(editor::Request {
            expected_revision: 0,
            actor: editor::Actor {
                kind: editor::ActorKind::Human,
                name: "Recovery test".into(),
            },
            action: editor::Action::AddNote {
                track: "ExpertSingle".into(),
                tick: 1200,
                lane: 2,
                sustain: 0,
                cymbal: false,
            },
        })
        .unwrap();
    let doc = store.load().unwrap();
    fn mount(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                mount(&entry.path(), &to.join(entry.file_name()));
            } else if entry.file_name() != "project.json" {
                storage::write_owned(
                    to.join(entry.file_name()),
                    std::fs::read(entry.path()).unwrap(),
                )
                .unwrap();
            }
        }
    }
    let root = PathBuf::from("/complete-project");
    let output = PathBuf::from("/complete-recovery");
    storage::clear(&root);
    storage::clear(&output);
    mount(&store.root, &root);
    let report = project_recovery::prepare(&root, &doc, &output).unwrap();
    assert!(report.source_error.is_none());
    assert!(report.missing_checkpoints.is_empty());
    let mut files = storage::list(&output)
        .unwrap()
        .into_iter()
        .map(|(p, b)| {
            (
                p.strip_prefix(&output)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
                b,
            )
        })
        .collect();
    let restored = temp.path().join("restored");
    project_portability::relocate(&mut files, &restored).unwrap();
    for (name, bytes) in files {
        let path = restored.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &*bytes).unwrap();
    }
    let restored = editor::Store::new(&restored);
    assert_eq!(restored.load().unwrap().revision, doc.revision);
    restored
        .apply(editor::Request {
            expected_revision: doc.revision,
            actor: editor::Actor {
                kind: editor::ActorKind::Human,
                name: "Recovery test".into(),
            },
            action: editor::Action::Undo,
        })
        .unwrap();
    assert!(
        !restored.load().unwrap().content.tracks["ExpertSingle"]
            .iter()
            .any(|n| n.note.tick == 1200)
    );
    storage::clear(&root);
    storage::clear(&output);
}
