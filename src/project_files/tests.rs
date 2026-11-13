use super::*;
use crate::editor::{Action, Actor, ActorKind, Request};

const CHART: &str = "\u{feff}[Song]\r\n{\r\n Resolution = 480\r\n Custom = keep\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n 0 = TS 7 3\r\n}\r\n[Events]\r\n{\r\n 120 = E \"opaque global text\"\r\n}\r\n[ExpertSingle]\r\n{\r\n 480 = N 0 120\r\n 480 = E \"opaque local text\"\r\n}\r\n[FutureInstrument]\r\n{\r\n 960 = X unknown-payload\r\n}\r\n";
const INI: &str = "[song]\r\nname = Preservation fixture\r\nartist = Test\r\ncustom_future_field = retain me\r\n[future]\r\nkey = unknown value\r\n";
fn fixture(root: &Path) -> Store {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    fs::write(song.join("song.ini"), INI).unwrap();
    fs::write(
        song.join("editor.lock"),
        b"authored song companion, not a live project lock",
    )
    .unwrap();
    fs::create_dir(song.join("extras")).unwrap();
    fs::write(song.join("extras/chart.lock"), b"opaque source data").unwrap();
    fs::write(
        song.join("extras/\u{97f3}\u{697d}.bin"),
        [0, 0xff, 0x10, 0x80],
    )
    .unwrap();
    Store::create(&song, None, &root.join("project")).unwrap()
}
fn action(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Preservation fixture".into(),
        },
        action,
    })?;
    Ok(())
}

#[test]
fn compact_backups_can_omit_generated_audio_without_losing_undo_history() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    action(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 1,
            sustain: 120,
            cymbal: false,
        },
    )
    .unwrap();
    let generated = store.root.join("source/.yace/audio/stems/key/guitar.wav");
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&generated, b"generated stem").unwrap();
    for include_generated_stems in [true, false] {
        let output = root
            .path()
            .join(format!("backup-{include_generated_stems}.zip"));
        backup_with_options(
            &store,
            &output,
            crate::compact_backup::Options {
                include_generated_stems,
                ..Default::default()
            },
        )
        .unwrap();
        let destination = root
            .path()
            .join(format!("restored-{include_generated_stems}"));
        let restored = import(&output, &destination).unwrap();
        assert_eq!(
            destination
                .join("source/.yace/audio/stems/key/guitar.wav")
                .exists(),
            include_generated_stems
        );
        action(&restored, Action::Undo).unwrap();
        assert_eq!(
            restored.load().unwrap().chart_text(&restored).unwrap(),
            CHART
        );
        action(&restored, Action::Redo).unwrap();
        assert!(
            restored
                .load()
                .unwrap()
                .chart_text(&restored)
                .unwrap()
                .contains("960 = N 1 120")
        );
    }
    assert!(generated.exists());
}

#[test]
fn backup_and_recovery_preserve_source_lock_suffixes_unknown_assets_and_edit_history() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    fs::write(store.root.join("custom.lock"), b"unknown project companion").unwrap();
    for name in [
        "editor.lock",
        "window.lock",
        "worker.lock",
        "vocal-worker.lock",
    ] {
        fs::write(store.root.join(name), b"session coordination").unwrap();
    }
    action(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 960,
            lane: 1,
            sustain: 120,
            cymbal: false,
        },
    )
    .unwrap();
    let output = root.path().join("backup.zip");
    backup(&store, &output).unwrap();
    let files: Files = archive_codec::decode(&fs::read(&output).unwrap())
        .unwrap()
        .into_iter()
        .collect();
    for (name, bytes) in [
        (
            "source/editor.lock",
            b"authored song companion, not a live project lock".as_slice(),
        ),
        ("source/extras/chart.lock", b"opaque source data".as_slice()),
        ("custom.lock", b"unknown project companion".as_slice()),
        (
            "source/extras/\u{97f3}\u{697d}.bin",
            [0, 0xff, 0x10, 0x80].as_slice(),
        ),
    ] {
        assert_eq!(
            files.get(name).map(Vec::as_slice),
            Some(bytes),
            "missing or changed {name}"
        );
    }
    for name in [
        "editor.lock",
        "window.lock",
        "worker.lock",
        "vocal-worker.lock",
    ] {
        assert!(!files.contains_key(name));
    }
    let restored = import(&output, &root.path().join("restored")).unwrap();
    action(&restored, Action::Undo).unwrap();
    assert_eq!(
        restored.load().unwrap().chart_text(&restored).unwrap(),
        CHART
    );
    action(&restored, Action::Redo).unwrap();
    let exported = root.path().join("export");
    action(
        &restored,
        Action::Export {
            output: exported.clone(),
        },
    )
    .unwrap();
    let text = fs::read_to_string(exported.join("notes.chart")).unwrap();
    assert!(text.contains("960 = N 1 120\r\n"));
    for fragment in [
        " Custom = keep\r\n",
        " 120 = E \"opaque global text\"\r\n",
        " 480 = E \"opaque local text\"\r\n",
        "[FutureInstrument]\r\n{\r\n 960 = X unknown-payload\r\n}\r\n",
    ] {
        assert!(text.contains(fragment), "{fragment}");
    }
    assert_eq!(
        fs::read_to_string(exported.join("song.ini"))
            .unwrap()
            .matches("custom_future_field = retain me\r\n")
            .count(),
        1
    );
    assert_eq!(
        fs::read(exported.join("extras/chart.lock")).unwrap(),
        b"opaque source data"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("song/notes.chart")).unwrap(),
        CHART
    );
    assert_eq!(
        fs::read_to_string(root.path().join("song/song.ini")).unwrap(),
        INI
    );
}

// APFS refuses this filename at creation (EILSEQ). The shared path guard above
// is still tested on macOS using an OsString without changing the filesystem.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn backup_refuses_non_utf8_filenames_without_publishing_a_renamed_copy() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let file = store
        .root
        .join("source/extras")
        .join(OsString::from_vec(b"opaque-\xff.bin".to_vec()));
    fs::write(&file, b"retain exact filename").unwrap();
    let output = root.path().join("refused.zip");
    assert!(
        backup(&store, &output)
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
    assert!(!output.exists());
    assert_eq!(fs::read(file).unwrap(), b"retain exact filename");
}

#[cfg(unix)]
#[test]
fn backup_refuses_literal_backslash_filenames_without_creating_a_different_directory() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let file = store.root.join("source/extras/opaque\\cue.bin");
    fs::write(&file, b"retain exact filename").unwrap();
    let output = root.path().join("refused.zip");
    assert!(
        backup(&store, &output)
            .unwrap_err()
            .to_string()
            .contains("literal backslash")
    );
    assert!(!output.exists());
    assert_eq!(fs::read(file).unwrap(), b"retain exact filename");
}

#[test]
fn unsupported_chart_to_midi_conversion_never_publishes_a_partial_song() {
    use crate::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let vocals = VocalChart {
        version: 1,
        resolution: 480,
        source: "Fixture".into(),
        phrases: vec![VocalPhrase {
            midi: None,
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
                text: "keep".into(),
                join_next: false,
            },
        }],
    };
    action(&store, Action::SetVocals { document: vocals }).unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    let output = root.path().join("refused");
    let failure = action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap_err();
    assert!(format!("{failure:#}").contains("Unsupported"));
    assert!(!output.exists());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(store.root.join("source/notes.chart")).unwrap(),
        CHART
    );
    assert_eq!(
        fs::read(store.root.join("source/extras/chart.lock")).unwrap(),
        b"opaque source data"
    );
}
