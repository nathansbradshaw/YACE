use chart_engine::vocals::VocalChart;
use std::{fs, path::Path, process::Command};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_chart-engine"))
}

#[test]
fn vocal_cli_round_trips_without_models_and_never_overwrites() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vocals-model-free.json");
    let midi = temporary.path().join("notes.mid");
    let export = || {
        command()
            .current_dir(temporary.path())
            .args(["vocals", "export"])
            .arg(&fixture)
            .args(["--bpm", "120", "--output"])
            .arg(&midi)
            .output()
            .unwrap()
    };
    let result = export();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let original = fs::read(&midi).unwrap();
    assert!(!export().status.success());
    assert_eq!(fs::read(&midi).unwrap(), original);
    let document = temporary.path().join("vocals.json");
    let result = command()
        .current_dir(temporary.path())
        .args(["vocals", "import"])
        .arg(&midi)
        .arg("--output")
        .arg(&document)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut expected: VocalChart = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    let loaded: VocalChart = serde_json::from_slice(&fs::read(&document).unwrap()).unwrap();
    expected.source = loaded.source.clone();
    assert_eq!(loaded, expected);
    let validation = command()
        .current_dir(temporary.path())
        .args(["vocals", "validate"])
        .arg(&document)
        .output()
        .unwrap();
    assert!(validation.status.success());
    let result: serde_json::Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(result["notes"], 5);
    assert!(!temporary.path().join("models").exists());

    let refused = temporary.path().join("refused.mid");
    let result = command()
        .current_dir(temporary.path())
        .args(["vocals", "export"])
        .arg(&document)
        .arg("--midi")
        .arg(&midi)
        .arg("--output")
        .arg(&refused)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!refused.exists());
    assert_eq!(fs::read(midi).unwrap(), original);
}

#[test]
fn vocal_export_cannot_create_competing_game_note_files() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vocals-model-free.json");
    for chart_name in ["notes.chart", "Notes.Chart"] {
        let temporary = tempfile::tempdir().unwrap();
        let chart = temporary.path().join(chart_name);
        fs::write(&chart, b"authored chart retained").unwrap();
        let midi = temporary.path().join("notes.mid");
        let result = command()
            .args(["vocals", "export"])
            .arg(&fixture)
            .args(["--bpm", "120", "--output"])
            .arg(&midi)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("competing"));
        assert!(!midi.exists());
        assert_eq!(fs::read(chart).unwrap(), b"authored chart retained");
    }
}

#[test]
fn vocal_review_import_uses_revision_guard_and_retains_notes_without_models() {
    use chart_engine::{
        editor::{Action, Actor, ActorKind, Request, Store},
        models,
        vocal_generation::{Generation, Issue, Policy, Reason, Status},
    };
    let temporary = tempfile::tempdir().unwrap();
    let song = temporary.path().join("song");
    fs::create_dir(&song).unwrap();
    let silence = vec![0.; 44100 * 5];
    models::write_audio(&song.join("song.wav"), &silence, &silence).unwrap();
    fs::write(song.join("notes.chart"), "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n").unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Synthetic\nartist = Test\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &temporary.path().join("project")).unwrap();
    let document: VocalChart = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vocals-model-free.json"),
        )
        .unwrap(),
    )
    .unwrap();
    store
        .apply(Request {
            expected_revision: 0,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Supplied vocal fixture".into(),
            },
            action: Action::SetVocals {
                document: document.clone(),
            },
        })
        .unwrap();
    let original = store.load().unwrap();
    let mut report = Generation {
        version: 1,
        audio_sha256: original.audio_sha256.clone(),
        vocal_stem_sha256: original.audio_sha256.clone(),
        duration_seconds: original.duration,
        timing: Some(original.timing(&store).unwrap()),
        algorithm: "Supplied synthetic review fixture".into(),
        policy: Policy::default(),
        status: Status::ReviewRequired,
        chart: Some(document.clone()),
        pitch_candidates: vec![],
        pitch_frames: vec![],
        words: None,
        lyrics: None,
        issues: vec![Issue {
            start_seconds: 0.5,
            end_seconds: 2.,
            reason: Reason::PitchDisagreement,
            detail: "Inspect this supplied synthetic contour".into(),
        }],
        warnings: vec![],
        neural_pitch: None,
        vocal_pitch: None,
    };
    let report_path = temporary.path().join("generation.json");
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let import = |revision: u64| {
        command()
            .current_dir(temporary.path())
            .args(["vocals", "attach-review", "--project"])
            .arg(&store.root)
            .arg("--report")
            .arg(&report_path)
            .arg("--expected-revision")
            .arg(revision.to_string())
            .output()
            .unwrap()
    };
    assert!(!import(0).status.success());
    assert_eq!(store.load().unwrap().revision, 1);
    assert!(store.load().unwrap().content.vocal_review.is_none());
    let applied = import(1);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(result["revision"], 2);
    let attached = store.load().unwrap();
    assert!(attached.content.vocal_review.is_some());
    assert_eq!(attached.content.vocals, Some(document));
    assert!(attached.content.decisions.is_empty());
    assert!(attached.content.hidden_flags.is_empty());
    report.audio_sha256 = "a".repeat(64);
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(!import(2).status.success());
    assert_eq!(store.load().unwrap().revision, attached.revision);
    assert!(!temporary.path().join("models").exists());
}
