use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    models,
    project_portability::{self, Files},
    stem_review::{AlignmentEvidence, StemId, StemManifest},
};
use std::{collections::BTreeMap, fs, path::Path};
fn command(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Portability test".into(),
            },
            action,
        })
        .unwrap();
}
fn fixture(root: &Path) -> (Store, std::path::PathBuf) {
    let source = root.join("song");
    fs::create_dir(&source).unwrap();
    let (ini, chart) =
        chart_engine::song_metadata::blank_song("Test title", "Test artist", 125.5).unwrap();
    fs::write(source.join("song.ini"), ini).unwrap();
    fs::write(source.join("notes.chart"), chart).unwrap();
    let tone: Vec<_> = (0..44100 * 3)
        .map(|i| (i as f32 * 0.03).sin() * 0.1)
        .collect();
    models::write_audio(&source.join("song.wav"), &tone, &tone).unwrap();
    let store = Store::create(&source, None, &root.join("project")).unwrap();
    let stem = root.join("external-drum.wav");
    let tone: Vec<_> = tone.iter().map(|v| v * 0.5).collect();
    models::write_audio(&stem, &tone, &tone).unwrap();
    let doc = store.load().unwrap();
    let manifest = StemManifest::build(
        &store.audio_path(&doc).unwrap(),
        BTreeMap::from([(StemId::Drums, stem.clone())]),
        AlignmentEvidence {
            method: "synthetic engineering fixture".into(),
            details: "Attenuated source; tests file transport, not instrument isolation".into(),
        },
    )
    .unwrap();
    let path = root.join("manifest.json");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    command(&store, Action::AttachStems { manifest: path });
    for tick in [480, 960] {
        command(
            &store,
            Action::AddNote {
                track: "ExpertSingle".into(),
                tick,
                lane: 0,
                sustain: 0,
                cymbal: false,
            },
        );
    }
    (store, stem)
}
fn collect(root: &Path, folder: &Path, files: &mut Files) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).unwrap(),
            );
        }
    }
}
fn bundle(store: &Store, stem: &Path) -> Files {
    let mut files = Files::new();
    collect(&store.root, &store.root, &mut files);
    files.insert("assets/drums.wav".into(), fs::read(stem).unwrap());
    files
}
fn write(root: &Path, files: &Files) {
    for (name, bytes) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

#[test]
fn imported_unedited_project_can_make_its_first_checkpoint_and_undo() {
    let temp = tempfile::tempdir().unwrap();
    let (original, _) = fixture(temp.path());
    let fresh = Store::create(
        &original.root.join("source"),
        None,
        &temp.path().join("fresh"),
    )
    .unwrap();
    let archive = temp.path().join("fresh.zip");
    chart_engine::project_files::backup(&fresh, &archive).unwrap();
    let imported =
        chart_engine::project_files::import(&archive, &temp.path().join("imported")).unwrap();
    assert!(!imported.root.join("checkpoints").exists());
    command(
        &imported,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 480,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    assert_eq!(
        imported.load().unwrap().content.tracks["ExpertSingle"].len(),
        1
    );
    command(&imported, Action::Undo);
    assert!(imported.load().unwrap().content.tracks["ExpertSingle"].is_empty());
}

#[test]
fn relocated_stems_and_content_addressed_history_survive_undo_and_redo() {
    let temp = tempfile::tempdir().unwrap();
    let (store, stem) = fixture(temp.path());
    let original = store.load().unwrap();
    let mut files = bundle(&store, &stem);
    let original_files = files.clone();
    let output = temp.path().join("moved");
    project_portability::relocate(&mut files, &output).unwrap();
    write(&output, &files);
    fs::remove_file(stem).unwrap();
    let moved = Store::new(&output);
    let doc = moved.load().unwrap();
    assert_eq!(
        doc.chart_hash(&moved).unwrap(),
        original.chart_hash(&store).unwrap()
    );
    assert_eq!(doc.audit.len(), original.audit.len());
    assert_eq!(doc.undo.len(), 3);
    assert_ne!(doc.undo, original.undo);
    assert_eq!(
        doc.content.stems.as_ref().unwrap().stems[0].path,
        output.join("assets/drums.wav")
    );
    doc.content
        .stems
        .as_ref()
        .unwrap()
        .load(&moved.audio_path(&doc).unwrap())
        .unwrap();
    for (name, bytes) in &original_files {
        if name.starts_with("source/") || name.starts_with("checkpoints/") {
            assert_eq!(files.get(name), Some(bytes));
        }
    }
    command(&moved, Action::Undo);
    assert_eq!(
        moved.load().unwrap().content.tracks["ExpertSingle"].len(),
        1
    );
    command(&moved, Action::Undo);
    assert!(moved.load().unwrap().content.tracks["ExpertSingle"].is_empty());
    command(&moved, Action::Undo);
    assert!(moved.load().unwrap().content.stems.is_none());
    command(&moved, Action::Redo);
    let doc = moved.load().unwrap();
    doc.content
        .stems
        .as_ref()
        .unwrap()
        .load(&moved.audio_path(&doc).unwrap())
        .unwrap();
    assert!(
        chart_engine::song_metadata::blank_song("Title\nInjected = value", "Artist", 120.).is_err()
    );
}

#[test]
fn missing_assets_cannot_read_old_absolute_paths_and_tampered_history_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let (store, stem) = fixture(temp.path());
    let mut files = bundle(&store, &stem);
    files.remove("assets/drums.wav");
    // Native attachments now retain their own content-addressed copy too.
    // Remove every archived copy while leaving the old machine's files intact.
    let stem_hash = models::hash_file(&stem).unwrap();
    files.retain(|_, bytes| {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes)) != stem_hash
    });
    let output = temp.path().join("missing-assets");
    project_portability::relocate(&mut files, &output).unwrap();
    let doc: chart_engine::editor::Project =
        serde_json::from_slice(&files["project.json"]).unwrap();
    assert!(
        doc.content.stems.as_ref().unwrap().stems[0]
            .path
            .starts_with(output.join("unavailable"))
    );
    let mut files = bundle(&store, &stem);
    let id = store.load().unwrap().undo.last().unwrap().clone();
    files.insert(format!("checkpoints/{id}.json"), b"changed".to_vec());
    assert!(project_portability::relocate(&mut files, &output).is_err());
}

#[cfg(feature = "editor")]
#[test]
fn native_backup_imports_a_browser_compatible_zip_without_overwriting_a_backup() {
    let temp = tempfile::tempdir().unwrap();
    let (store, stem) = fixture(temp.path());
    let archive = temp.path().join("project.zip");
    chart_engine::project_files::backup(&store, &archive).unwrap();
    let original = fs::read(&archive).unwrap();
    let stem_hash = models::hash_file(&stem).unwrap();
    let copies = chart_engine::archive_codec::decode(&original)
        .unwrap()
        .iter()
        .filter(|(_, bytes)| {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(bytes)) == stem_hash
        })
        .count();
    assert_eq!(
        copies, 1,
        "Retained stems must not be duplicated in portable backups"
    );
    assert!(chart_engine::project_files::backup(&store, &archive).is_err());
    assert_eq!(fs::read(&archive).unwrap(), original);
    fs::remove_file(stem).unwrap();
    let moved =
        chart_engine::project_files::import(&archive, &temp.path().join("imported")).unwrap();
    let doc = moved.load().unwrap();
    assert_eq!(doc.content.tracks["ExpertSingle"].len(), 2);
    doc.content
        .stems
        .as_ref()
        .unwrap()
        .load(&moved.audio_path(&doc).unwrap())
        .unwrap();
    command(&moved, Action::Undo);
    assert_eq!(
        moved.load().unwrap().content.tracks["ExpertSingle"].len(),
        1
    );
}
