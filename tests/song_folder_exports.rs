use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store},
    models, pipeline, song_layout,
};
use std::{collections::BTreeSet, fs, path::Path};

fn song(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("notes.chart"), "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n}\n").unwrap();
    fs::write(
        root.join("song.ini"),
        "[song]\nname = Organized\nartist = Fixture\n",
    )
    .unwrap();
    models::write_audio(&root.join("song.wav"), &vec![0.; 88200], &vec![0.; 88200]).unwrap();
}

#[test]
fn managed_project_snapshot_does_not_copy_itself_or_mutable_song_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Song");
    song(&source);
    for name in [
        "jobs/old-job",
        "cache/decoded",
        "logs/worker",
        "projects/old-copy",
        ".tmpstage",
    ] {
        let path = source.join(".yace").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"Mutable state must not become a song asset").unwrap();
    }
    let original = fs::read(source.join("notes.chart")).unwrap();
    let output = source.join(".yace/project");
    let store = Store::create(&source, None, &output).unwrap();
    assert_eq!(fs::read(source.join("notes.chart")).unwrap(), original);
    assert!(store.root.join("source/notes.chart").is_file());
    for name in ["project", "projects", "jobs", "cache", "logs", ".tmpstage"] {
        assert!(
            !store.root.join("source/.yace").join(name).exists(),
            "Copied mutable {name}"
        );
    }
    assert_eq!(
        store.load().unwrap().original_folder,
        source.canonicalize().unwrap()
    );
    assert_eq!(store.root, output.canonicalize().unwrap());
}

#[test]
fn exported_song_keeps_playable_files_visible_and_groups_yace_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Song");
    song(&source);
    fs::write(source.join("album.png"), b"Artwork fixture").unwrap();
    fs::write(source.join("custom.cfg"), b"Unknown user file").unwrap();
    let old_evidence = b"{\"source\":\"legacy lyric evidence\"}";
    fs::write(source.join("chart-engine-lyrics.json"), old_evidence).unwrap();
    let store = Store::create(&source, None, &temp.path().join("legacy-project")).unwrap();
    let output = temp.path().join("Export");
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Layout test".into(),
            },
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let names: BTreeSet<_> = fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            ".yace",
            "notes.chart",
            "song.ini",
            "song.wav",
            "album.png",
            "custom.cfg"
        ]
        .into_iter()
        .map(String::from)
        .collect()
    );
    assert!(song_layout::path(&output, "editor-review.json").is_file());
    assert!(song_layout::path(&output, "game-export.json").is_file());
    assert!(song_layout::path(&output, "yace-audio.json").is_file());
    assert_eq!(
        fs::read(output.join(".yace/history/editor-source/chart-engine-lyrics.json")).unwrap(),
        old_evidence
    );
    assert_eq!(
        fs::read(source.join("chart-engine-lyrics.json")).unwrap(),
        old_evidence
    );
    assert_eq!(
        fs::read(output.join("custom.cfg")).unwrap(),
        b"Unknown user file"
    );
    let reopened = Store::create(&output, None, &temp.path().join("Reopened")).unwrap();
    assert_eq!(
        reopened.load().unwrap().content.tracks,
        store.load().unwrap().content.tracks
    );
}

#[test]
fn copied_legacy_layout_moves_only_yace_owned_entries_and_keeps_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Source");
    let stage = temp.path().join("Stage");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("chart-engine-report.json"), b"old receipt").unwrap();
    fs::write(source.join("notes.chart"), b"standard chart").unwrap();
    fs::create_dir_all(source.join(".yace/analysis")).unwrap();
    fs::write(
        song_layout::path(&source, "chart-engine-report.json"),
        b"current receipt",
    )
    .unwrap();
    pipeline::copy_song_source(&source, &stage).unwrap();
    pipeline::organize_song_stage(&stage).unwrap();
    assert_eq!(
        fs::read(song_layout::path(&stage, "chart-engine-report.json")).unwrap(),
        b"current receipt"
    );
    assert_eq!(
        fs::read(stage.join(".yace/history/legacy/chart-engine-report.json")).unwrap(),
        b"old receipt"
    );
    assert_eq!(
        fs::read(stage.join("notes.chart")).unwrap(),
        b"standard chart"
    );
    assert!(!stage.join("chart-engine-report.json").exists());
    assert!(source.join("chart-engine-report.json").exists());
}
