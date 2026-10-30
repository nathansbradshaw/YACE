use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    game_export,
    models::hash_file,
    timing::TempoMap,
    vocal_generation::{ALGORITHM, Generation, PitchedSpan, Policy, Status},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn json_file(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn wave(path: &Path) {
    let frames = 88_200u32;
    let bytes = frames * 8;
    let mut data = Vec::new();
    data.extend(b"RIFF");
    data.extend((bytes + 36).to_le_bytes());
    data.extend(b"WAVEfmt ");
    data.extend(16u32.to_le_bytes());
    data.extend(3u16.to_le_bytes());
    data.extend(2u16.to_le_bytes());
    data.extend(44100u32.to_le_bytes());
    data.extend((44100u32 * 8).to_le_bytes());
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
fn vocal_stage_failure_retains_source_bound_band_artifact_and_never_scores_partial_vocals() {
    let temp = tempfile::tempdir().unwrap();
    let engine = temp.path().join("fake-engine");
    assert!(
        Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "--crate-name", "benchmark_failure_engine"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/failing_engine.rs"))
            .arg("-o")
            .arg(&engine)
            .status()
            .unwrap()
            .success()
    );
    let engine_hash = hash_file(&engine).unwrap();
    let batch = temp.path().join("batch");
    fs::create_dir(&batch).unwrap();
    fs::write(
        batch.join("frozen-batch.json"),
        serde_json::to_vec(&json!({"songs":[{"archive_sha256":"a".repeat(64)}]})).unwrap(),
    )
    .unwrap();
    let case = batch.join("heldout-01");
    fs::create_dir(&case).unwrap();
    wave(&case.join("source.wav"));
    fs::copy(case.join("source.wav"), case.join("bass.wav")).unwrap();
    fs::copy(case.join("source.wav"), case.join("vocals.wav")).unwrap();
    fs::write(case.join("song.ini"), b"[song]\ndelay = 0\n").unwrap();
    let timing = TempoMap::constant(120.).unwrap();
    let tracks: Tracks = ["ExpertSingle", "ExpertDoubleBass", "ExpertDrums"]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                vec![Note {
                    tick: 192,
                    lane: 1,
                    sustain: 0,
                }],
            )
        })
        .collect();
    let chart_text = chart::new_chart("Synthetic partial failure", "Fixture", &timing, &tracks);
    let chart_path = temp.path().join("fixture.chart");
    fs::write(&chart_path, &chart_text).unwrap();
    let vocal = VocalChart {
        version: 1,
        resolution: timing.resolution,
        source: "Synthetic fixture".into(),
        phrases: vec![VocalPhrase {
            start: 192,
            end: 384,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 192,
            end: 288,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "test".into(),
                join_next: false,
            },
        }],
    };
    let vocal_path = temp.path().join("fixture-vocal.json");
    fs::write(&vocal_path, serde_json::to_vec(&vocal).unwrap()).unwrap();
    let (midi, _) = game_export::song(&Chart::parse(chart_text).unwrap(), Some(&vocal)).unwrap();
    fs::write(case.join("notes.mid"), midi).unwrap();
    let assets: Vec<_> = ["bass.wav", "vocals.wav", "notes.mid"].into_iter().map(|name| json!({"name":name,
        "bytes":fs::metadata(case.join(name)).unwrap().len(),"sha256":hash_file(&case.join(name)).unwrap()})).collect();
    json_file(
        &case.join("reference-source.json"),
        &json!({"version":1,"assets":assets,
        "metadata":{"delay":"0"},"song_ini_sha256":hash_file(&case.join("song.ini")).unwrap(),"source_archive_sha256":"a".repeat(64)}),
    );
    json_file(
        &case.join("source.mix.json"),
        &json!({"version":1,"output_sha256":hash_file(&case.join("source.wav")).unwrap(),
        "source_archive_sha256":"a".repeat(64),"sample_rate":44100,"channels":2,"frames":88200,
        "duration_seconds":2.,"ini_delay_milliseconds":0.,"included_stems":[{"stem":"bass",
        "sha256":hash_file(&case.join("bass.wav")).unwrap(),"frames":88200,"starts_at_source_frame":0},
        {"stem":"vocals","sha256":hash_file(&case.join("vocals.wav")).unwrap(),"frames":88200,"starts_at_source_frame":0}]}),
    );
    let oracle = temp.path().join("oracle");
    chart_benchmark::prepare(&case, &oracle).unwrap();
    let report = Generation {
        version: 1,
        audio_sha256: hash_file(&case.join("source.wav")).unwrap(),
        vocal_stem_sha256: hash_file(&oracle.join("vocals.wav")).unwrap(),
        duration_seconds: 2.,
        algorithm: ALGORITHM.into(),
        timing: Some(timing.clone()),
        policy: Policy::default(),
        status: Status::ReviewRequired,
        chart: Some(vocal.clone()),
        pitch_candidates: vec![PitchedSpan {
            start_seconds: 0.2,
            end_seconds: 0.3,
            midi: 60,
            clarity: 0.9,
        }],
        pitch_frames: vec![],
        words: None,
        lyrics: None,
        issues: vec![],
        warnings: vec![],
        neural_pitch: None,
        vocal_pitch: None,
    };
    let models = temp.path().join("models");
    fs::create_dir_all(models.join("vocal-alignment/en")).unwrap();
    for name in [
        "basic_pitch.onnx",
        "adtof_frame_rnn.onnx",
        "swift-f0-v0.3.0.onnx",
        "vocal-alignment/en/config.json",
        "vocal-alignment/en/vocab.json",
        "vocal-alignment/en/model.safetensors",
    ] {
        fs::write(
            models.join(name),
            b"synthetic model identity, never executed",
        )
        .unwrap();
    }
    for mode in [
        "failed-with-partial-chart",
        "missing",
        "invalid",
        "incompatible-export",
        "missing-report",
        "corrupt-report",
        "wrong-source-report",
        "wrong-stem-report",
        "wrong-clock-report",
        "wrong-timing-report",
        "contradictory-abstention",
        "needs-lyrics",
        "no-voiced-pitch",
        "model-free-abstention",
        "complete",
    ] {
        let runs = temp.path().join(mode);
        fs::create_dir(&runs).unwrap();
        let output = runs.join("heldout-01");
        let abstained = matches!(
            mode,
            "needs-lyrics" | "no-voiced-pitch" | "model-free-abstention"
        );
        let model_free = mode == "model-free-abstention";
        let success = mode == "complete" || abstained;
        let mut report = report.clone();
        if abstained || mode == "contradictory-abstention" {
            report.chart = None;
            report.status = if mode == "no-voiced-pitch" {
                report.pitch_candidates.clear();
                Status::NoVoicedPitch
            } else {
                Status::NeedsLyrics
            };
        }
        match mode {
            "wrong-source-report" => report.audio_sha256 = "b".repeat(64),
            "wrong-stem-report" => report.vocal_stem_sha256 = "b".repeat(64),
            "wrong-clock-report" => report.duration_seconds = 2.1,
            "wrong-timing-report" => report.timing = Some(TempoMap::constant(100.).unwrap()),
            _ => {}
        }
        let report_path = runs.join("report-input.json");
        fs::write(
            &report_path,
            if mode == "corrupt-report" {
                b"corrupt".to_vec()
            } else {
                serde_json::to_vec(&report).unwrap()
            },
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_chart-benchmark"));
        command
            .arg("run")
            .arg(&engine)
            .arg(&case)
            .arg(&oracle)
            .arg(&output)
            .arg(&models)
            .arg(if model_free {
                "model-free-vocals"
            } else {
                "oracle"
            })
            .env("FAKE_ENGINE_CHART", &chart_path)
            .env("FAKE_ENGINE_VOCAL_CHART", &vocal_path)
            .env("FAKE_ENGINE_REPORT", &report_path)
            .env("FAKE_ENGINE_VOCALS", mode);
        if model_free {
            command.env("FAKE_ENGINE_MODEL_FREE", "1");
        }
        let status = command.output().unwrap();
        assert_eq!(
            status.status.success(),
            success,
            "{mode}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        let run = read(&output.join("run.json"));
        assert_eq!(run["success"], success);
        if model_free {
            assert_eq!(run["stages"].as_array().unwrap().len(), 1);
            assert!(
                run["instrument_outcomes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|o| o["instrument"] != "vocals")
                    .all(|o| o["status"] == "not_run")
            );
        }
        assert_eq!(run["engine_sha256"], engine_hash);
        assert_eq!(
            run["artifact_sha256"],
            hash_file(&output.join("generated.mid")).unwrap()
        );
        assert_eq!(
            run["instrument_outcomes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| o["success"] == true)
                .count(),
            if model_free {
                1
            } else if success {
                5
            } else {
                4
            }
        );
        let generated = fs::read(output.join("generated.mid")).unwrap();
        assert_eq!(
            generated.windows(11).any(|w| w == b"PART VOCALS"),
            mode == "complete"
        );
        json_file(
            &runs.join("batch-run.json"),
            &json!({"complete":true,"expected_cases":1,"condition":"synthetic",
            "frozen_batch_sha256":hash_file(&batch.join("frozen-batch.json")).unwrap()}),
        );
        let evaluation = runs.join("evaluation");
        chart_benchmark::evaluate::evaluate(&batch, &runs, &evaluation).unwrap();
        let results = read(&evaluation.join("heldout-01.json"))["results"]
            .as_array()
            .unwrap()
            .clone();
        for result in &results {
            assert_eq!(
                result["success"],
                if result["instrument"] == "vocals" {
                    success
                } else {
                    !model_free
                },
                "{mode}: {result}"
            );
        }
        if abstained {
            let vocal = results
                .iter()
                .find(|r| r["instrument"] == "vocals")
                .unwrap();
            for metric in vocal["report"]["attacks"].as_array().unwrap() {
                assert!(metric["reference_attacks"].as_u64().unwrap() > 0);
                assert_eq!(metric["generated_attacks"], 0);
                assert_eq!(metric["recall"], 0.);
                assert!(metric["precision"].is_null());
            }
            let summary = read(&evaluation.join("summary.json"));
            let vocal = summary["instruments"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["instrument"] == "vocals")
                .unwrap();
            assert_eq!(vocal["scored_cases"], 1);
            assert_eq!(vocal["attacks"][0]["micro_recall"], 0.);
            assert_eq!(vocal["attacks"][0]["micro_f1"], 0.);
            assert!(vocal["attacks"][0]["micro_precision"].is_null());
            let generation_path = output.join("vocals/generation.json");
            fs::write(&generation_path, b"changed after inference").unwrap();
            let evaluation = runs.join("changed-abstention-evidence");
            chart_benchmark::evaluate::evaluate(&batch, &runs, &evaluation).unwrap();
            let report = read(&evaluation.join("heldout-01.json"));
            let vocal = report["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["instrument"] == "vocals")
                .unwrap();
            assert_eq!(vocal["success"], false);
            assert!(
                vocal["error"]
                    .as_str()
                    .unwrap()
                    .contains("Abstention report changed")
            );
        }
        if mode == "failed-with-partial-chart" {
            // A per-instrument success declaration cannot override the actual
            // failed subprocess stage, even if a partial chart exists.
            let mut forged = run.clone();
            for outcome in forged["instrument_outcomes"].as_array_mut().unwrap() {
                outcome["success"] = json!(true);
            }
            json_file(&output.join("run.json"), &forged);
            let evaluation = runs.join("failed-stage-cannot-be-laundered");
            chart_benchmark::evaluate::evaluate(&batch, &runs, &evaluation).unwrap();
            let report = read(&evaluation.join("heldout-01.json"));
            let vocal = report["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|outcome| outcome["instrument"] == "vocals")
                .unwrap();
            assert_eq!(vocal["success"], false);
            assert!(
                vocal["error"]
                    .as_str()
                    .unwrap()
                    .contains("Required vocals stage did not succeed")
            );
        }
    }
    assert_eq!(hash_file(&engine).unwrap(), engine_hash);
}
