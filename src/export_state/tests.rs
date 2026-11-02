use super::*;
use crate::editor::{Action, Actor, ActorKind, Request};

fn fixture(root: &std::path::Path) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"),"[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n480 = N 0 120\n}\n").unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Export status fixture\ncustom = keep\nsong_length = 4000\n",
    )
    .unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn action(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Export state fixture".into(),
            },
            action,
        })
        .unwrap();
}
fn receipt(store: &Store, delivery: Delivery) -> Receipt {
    Receipt::capture(
        store,
        &store.load().unwrap(),
        &Options::default(),
        "test song copy".into(),
        delivery,
    )
    .unwrap()
}

#[test]
fn history_does_not_change_revision_and_transport_does_not_make_the_song_stale() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = fs::read(store.root.join("project.json")).unwrap();
    let record = receipt(&store, Delivery::PublishedFolder);
    record.save(&store).unwrap();
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    action(&store, Action::Seek { seconds: 0.5 });
    action(&store, Action::SetPlaybackRate { rate: 0.75 });
    action(
        &store,
        Action::SelectTrack {
            track: "ExpertSingle".into(),
        },
    );
    let doc = store.load().unwrap();
    assert!(doc.revision > record.revision);
    assert_eq!(fingerprint(&store, &doc).unwrap(), record.fingerprint);
    assert!(
        State::load(&store, &doc)
            .summary()
            .contains("editing content matches")
    );
    assert!(
        State::load(&store, &doc)
            .summary()
            .contains("without audio")
    );
}

#[test]
#[cfg(any(feature = "editor", feature = "browser"))]
fn edits_and_undo_distinguish_current_and_older_exports_and_recovery_keeps_the_receipt() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let record = receipt(&store, Delivery::PublishedFolder);
    record.save(&store).unwrap();
    action(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    assert!(
        State::load(&store, &store.load().unwrap())
            .summary()
            .contains("project changed")
    );
    let backup = root.path().join("backup.zip");
    crate::project_files::backup(&store, &backup).unwrap();
    let moved = crate::project_files::import(&backup, &root.path().join("restored")).unwrap();
    assert!(
        State::load(&moved, &moved.load().unwrap())
            .summary()
            .contains("project changed")
    );
    action(&moved, Action::Undo);
    assert_eq!(
        fingerprint(&moved, &moved.load().unwrap()).unwrap(),
        record.fingerprint
    );
    assert_eq!(
        Receipt::load(&moved, &moved.load().unwrap()).unwrap(),
        Some(record)
    );
    action(&moved, Action::Redo);
    assert!(
        State::load(&moved, &moved.load().unwrap())
            .summary()
            .contains("project changed")
    );
}

#[test]
fn browser_preparation_and_download_request_never_claim_a_file_was_saved() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut record = receipt(&store, Delivery::PreparedZip);
    let prepared = record.summary(&record.fingerprint);
    assert!(prepared.contains("not downloaded"));
    assert!(!prepared.contains("saved"));
    record.request_download().unwrap();
    record.save(&store).unwrap();
    let text = State::load(&store, &store.load().unwrap()).summary();
    assert!(text.contains("download requested"));
    assert!(!text.contains("saved"));
    let mut folder = receipt(&store, Delivery::PublishedFolder);
    assert!(folder.request_download().is_err());
}

#[test]
fn damaged_history_stays_intact_and_does_not_turn_a_successful_export_into_a_failed_copy() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    assert!(fs::is_file(output.join("notes.chart")));
    let path = store.root.join(FILE);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not a receipt").unwrap();
    assert!(
        receipt(&store, Delivery::PublishedFolder)
            .save(&store)
            .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), b"not a receipt");
    assert!(fs::is_file(output.join("notes.chart")));
    assert!(State::load(&store, &store.load().unwrap()).error.is_some());
}

#[test]
fn source_metadata_and_recording_identity_are_part_of_the_export_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let doc = store.load().unwrap();
    let original = fingerprint(&store, &doc).unwrap();
    fs::write(
        store.root.join("source/song.ini"),
        "[song]\nname = Changed metadata\n",
    )
    .unwrap();
    assert_ne!(fingerprint(&store, &doc).unwrap(), original);
    let changed_metadata = fingerprint(&store, &doc).unwrap();
    let mut changed = doc.clone();
    changed.audio_sha256 = "b".repeat(64);
    assert_ne!(fingerprint(&store, &changed).unwrap(), changed_metadata);
}

#[test]
fn native_publication_records_its_snapshot_and_a_refused_export_cannot_promote_it() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let output = root.path().join("song-copy");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let before = Receipt::load(&store, &store.load().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(before.delivery, Delivery::PublishedFolder);
    assert_eq!(
        before.fingerprint,
        fingerprint(&store, &store.load().unwrap()).unwrap()
    );
    let exported = fs::read(output.join("notes.chart")).unwrap();
    action(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    let result = store.apply(Request {
        expected_revision: store.load().unwrap().revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Export fixture".into(),
        },
        action: Action::Export {
            output: output.clone(),
        },
    });
    assert!(result.is_err());
    assert_eq!(
        Receipt::load(&store, &store.load().unwrap()).unwrap(),
        Some(before)
    );
    assert_eq!(fs::read(output.join("notes.chart")).unwrap(), exported);
    assert!(
        State::load(&store, &store.load().unwrap())
            .summary()
            .contains("project changed")
    );
}
