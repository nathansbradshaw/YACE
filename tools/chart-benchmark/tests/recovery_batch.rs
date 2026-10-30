use chart_engine::{
    chart::{self, Note, Tracks},
    mapping::{PitchNote, Transcription},
    models::hash_file,
    timing::TempoMap,
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn save(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn load(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn wave(path: &Path) {
    let frames = 88200u32;
    let bytes = frames * 8;
    let mut data = Vec::new();
    data.extend(b"RIFF");
    data.extend((bytes + 36).to_le_bytes());
    data.extend(b"WAVEfmt ");
    data.extend(16u32.to_le_bytes());
    data.extend(3u16.to_le_bytes());
    data.extend(2u16.to_le_bytes());
    data.extend(44100u32.to_le_bytes());
    data.extend(352800u32.to_le_bytes());
    data.extend(8u16.to_le_bytes());
    data.extend(32u16.to_le_bytes());
    data.extend(b"data");
    data.extend(bytes.to_le_bytes());
    for _ in 0..frames {
        data.extend(0.1f32.to_le_bytes());
        data.extend(0.1f32.to_le_bytes());
    }
    fs::write(path, data).unwrap();
}
#[test]
fn one_audit_per_frozen_case_retains_failures_and_sanitizes_paired_results() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let engine = root.join("audit");
    assert!(
        Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "--crate-name", "synthetic_recovery_audit"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recovery_audit.rs"))
            .arg("-o")
            .arg(&engine)
            .status()
            .unwrap()
            .success()
    );
    let prepared = root.join("prepared");
    fs::create_dir(&prepared).unwrap();
    save(
        &prepared.join("frozen-batch.json"),
        &json!({"songs":(0..6).map(|_|json!({"archive_sha256":"a".repeat(64)})).collect::<Vec<_>>()}),
    );
    let template = root.join("template");
    fs::create_dir(&template).unwrap();
    let timing = TempoMap::constant(120.).unwrap();
    for revised in [false, true] {
        let mut tracks = Tracks::new();
        for name in ["ExpertSingle", "ExpertDoubleBass", "ExpertDrums"] {
            let lane = if name == "ExpertDrums" { 2 } else { 0 };
            let mut notes = vec![Note {
                tick: 192,
                lane,
                sustain: if name == "ExpertSingle" { 192 } else { 0 },
            }];
            if name == "ExpertDrums" {
                notes.push(Note {
                    tick: 192,
                    lane: 66,
                    sustain: 0,
                });
            }
            if revised && name != "ExpertDoubleBass" {
                notes.push(Note {
                    tick: 384,
                    lane: 1,
                    sustain: 0,
                });
            }
            tracks.insert(name.into(), notes);
        }
        let variant = if revised { "revised" } else { "baseline" };
        fs::write(
            template.join(format!("{variant}.chart")),
            chart::new_chart(
                "PRIVATE synthetic title",
                "PRIVATE synthetic artist",
                &timing,
                &tracks,
            ),
        )
        .unwrap();
        let events = Transcription {
            bass: vec![PitchNote {
                start: 0.2,
                end: 0.4,
                pitch: 40,
                confidence: 0.9,
            }],
            ..Default::default()
        };
        save(
            &template.join(format!("{variant}-events.json")),
            &serde_json::to_value(&events).unwrap(),
        );
        if revised {
            let mut bad = events;
            bad.bass[0].pitch = 41;
            save(
                &template.join("bad-bass-events.json"),
                &serde_json::to_value(bad).unwrap(),
            );
        }
    }
    for index in 1..=6 {
        let case = prepared.join(format!("heldout-{index:02}"));
        fs::create_dir(&case).unwrap();
        wave(&case.join("source.wav"));
        for role in ["bass", "guitar", "drums", "vocals"] {
            fs::copy(case.join("source.wav"), case.join(format!("{role}.wav"))).unwrap();
        }
        fs::copy(template.join("revised.chart"), case.join("notes.chart")).unwrap();
        fs::write(case.join("song.ini"), b"[song]\ndelay = 0\n").unwrap();
        let assets:Vec<_>=["bass.wav","guitar.wav","drums.wav","vocals.wav","notes.chart"].into_iter().map(|name|json!({"name":name,
            "bytes":fs::metadata(case.join(name)).unwrap().len(),"sha256":hash_file(&case.join(name)).unwrap()})).collect();
        save(
            &case.join("reference-source.json"),
            &json!({"version":1,"assets":assets,"metadata":{"delay":"0"},
            "song_ini_sha256":hash_file(&case.join("song.ini")).unwrap(),"source_archive_sha256":"a".repeat(64)}),
        );
        let stems:Vec<_>=["bass","guitar","drums","vocals"].into_iter().map(|role|json!({"stem":role,"sha256":hash_file(&case.join(format!("{role}.wav"))).unwrap(),"frames":88200,"starts_at_source_frame":0})).collect();
        save(
            &case.join("source.mix.json"),
            &json!({"version":1,"output_sha256":hash_file(&case.join("source.wav")).unwrap(),
            "source_archive_sha256":"a".repeat(64),"sample_rate":44100,"channels":2,"frames":88200,"duration_seconds":2.,"ini_delay_milliseconds":0.,"included_stems":stems}),
        );
    }
    let oracle = root.join("oracle-template");
    chart_benchmark::prepare(&prepared.join("heldout-01"), &oracle).unwrap();
    let comparison = json!({"version":1,"audio_sha256":hash_file(&prepared.join("heldout-01/source.wav")).unwrap(),"stem_manifest_sha256":hash_file(&oracle.join("stem-manifest.json")).unwrap()});
    save(&template.join("comparison.json"), &comparison);
    let mut wrong = comparison;
    wrong["audio_sha256"] = json!("b".repeat(64));
    save(&template.join("wrong-source-comparison.json"), &wrong);
    let models = root.join("models");
    fs::create_dir(&models).unwrap();
    for name in ["basic_pitch.onnx", "adtof_frame_rnn.onnx"] {
        fs::write(models.join(name), b"test identity, never executed").unwrap();
    }
    let output = root.join("result");
    let calls = root.join("calls.log");
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_chart-benchmark"))
            .arg("recovery-batch")
            .arg(&engine)
            .arg(&prepared)
            .arg(&output)
            .arg(&models)
            .env("FAKE_RECOVERY_TEMPLATE", &template)
            .env("FAKE_RECOVERY_CALLS", &calls)
            .output()
            .unwrap()
    };
    let status = invoke();
    assert!(!status.status.success());
    let summary = load(&output.join("summary.json"));
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["all_cases_successful"], false);
    assert_eq!(summary["cases"].as_array().unwrap().len(), 6);
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 6);
    for entry in summary["instruments"].as_array().unwrap() {
        assert_eq!(entry["baseline"]["scored_cases"], 3);
        assert_eq!(entry["baseline"]["failed_cases"], 3);
        assert_eq!(entry["paired_cases"], 3);
        let before = &entry["baseline"]["metrics"]["attacks"][1];
        let after = &entry["revised"]["metrics"]["attacks"][1];
        if entry["instrument"] == "bass" {
            assert_eq!(before, after);
        } else {
            assert_eq!(before["matched"], 3);
            assert_eq!(after["matched"], 6);
            assert_eq!(after["micro_recall"], 1.);
        }
    }
    let first = &summary["cases"][0];
    assert_eq!(first["recovery"]["bass_unchanged"], true);
    assert_eq!(first["recovery"]["drums"]["baseline_gems"], 1); // cymbal flag is not a gem
    assert_eq!(first["recovery"]["guitar"]["added_heads"], 1);
    let serialized = serde_json::to_string(&summary).unwrap();
    assert!(!serialized.contains(root.to_str().unwrap()));
    assert!(!serialized.contains("PRIVATE synthetic"));
    for index in 1..=6 {
        assert!(
            output
                .join(format!("heldout-{index:02}/case.json"))
                .exists()
        );
    }
    let bytes = fs::read(output.join("summary.json")).unwrap();
    assert!(!invoke().status.success());
    assert_eq!(fs::read(output.join("summary.json")).unwrap(), bytes);
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 6);
}
