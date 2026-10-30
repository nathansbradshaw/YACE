use chart_benchmark::prepare;
use chart_engine::{
    chart::Tracks,
    midi,
    models::hash_file,
    stem_review::StemManifest,
    timing::TempoMap,
    vocals::{self, Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use serde_json::{Value, json};
use std::{fs, path::Path};
fn wave(path: &Path, frames: u32) {
    let bytes = frames * 8;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((bytes + 36).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(44100u32.to_le_bytes());
    b.extend((44100u32 * 8).to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend(bytes.to_le_bytes());
    for _ in 0..frames {
        b.extend(0.125f32.to_le_bytes());
        b.extend((-0.25f32).to_le_bytes());
    }
    fs::write(path, b).unwrap();
}
fn fixture(root: &Path) {
    fs::create_dir(root).unwrap();
    wave(&root.join("source.wav"), 1000);
    wave(&root.join("bass.wav"), 600);
    fs::write(root.join("song.ini"), b"[song]\ndelay = 25\n").unwrap();
    let timing = TempoMap::constant(120.).unwrap();
    let base = midi::new_midi(&timing, &Tracks::new()).unwrap();
    let chart = VocalChart {
        version: 1,
        resolution: 480,
        source: "Synthetic text fixture".into(),
        phrases: vec![VocalPhrase {
            start: 0,
            end: 10,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 0,
            end: 10,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "test".into(),
                join_next: false,
            },
        }],
    };
    fs::write(
        root.join("notes.mid"),
        vocals::append_midi(&base, &chart).unwrap(),
    )
    .unwrap();
    let assets:Vec<_>=["bass.wav","notes.mid"].into_iter().map(|name|json!({"name":name,"bytes":fs::metadata(root.join(name)).unwrap().len(),"sha256":hash_file(&root.join(name)).unwrap()})).collect();
    let reference = json!({"version":1,"assets":assets,"metadata":{"delay":"25"},"song_ini_sha256":hash_file(&root.join("song.ini")).unwrap(),"source_archive_sha256":"a".repeat(64)});
    let mix = json!({"version":1,"output_sha256":hash_file(&root.join("source.wav")).unwrap(),"source_archive_sha256":"a".repeat(64),"sample_rate":44100,"channels":2,"frames":1000,"duration_seconds":1000./44100.,"ini_delay_milliseconds":25.,"included_stems":[{"stem":"bass","sha256":hash_file(&root.join("bass.wav")).unwrap(),"frames":600,"starts_at_source_frame":0}]});
    fs::write(
        root.join("reference-source.json"),
        serde_json::to_vec_pretty(&reference).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("source.mix.json"),
        serde_json::to_vec_pretty(&mix).unwrap(),
    )
    .unwrap();
}
fn mutate(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut v: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut v);
    fs::write(path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}
#[test]
fn published_manifest_is_relocatable_typed_and_preserves_unscaled_samples_and_zero_tail() {
    let temp = tempfile::tempdir().unwrap();
    let case = temp.path().join("case");
    fixture(&case);
    let original = fs::read(case.join("bass.wav")).unwrap();
    let output = temp.path().join("oracle");
    prepare(&case, &output).unwrap();
    let raw: Value =
        serde_json::from_slice(&fs::read(output.join("stem-manifest.json")).unwrap()).unwrap();
    assert_eq!(raw["stems"][0]["path"], "bass.wav");
    let manifest = StemManifest::read(&output.join("stem-manifest.json")).unwrap();
    manifest
        .validate_binding(
            &hash_file(&case.join("source.wav")).unwrap(),
            1000. / 44100.,
        )
        .unwrap();
    assert!(
        manifest.stems[0]
            .path
            .starts_with(output.canonicalize().unwrap())
    );
    let decoded = chart_engine::models::decode(&manifest.stems[0].path, 44100, 2).unwrap();
    assert_eq!(decoded.len(), 2000);
    assert!(decoded[..1200].chunks_exact(2).all(|n| n == [0.125, -0.25]));
    assert!(decoded[1200..].iter().all(|s| *s == 0.));
    assert_eq!(fs::read(case.join("bass.wav")).unwrap(), original);
    let lyrics: Value =
        serde_json::from_slice(&fs::read(output.join("lyric-source.json")).unwrap()).unwrap();
    assert_eq!(lyrics["text"], "test");
    assert!(lyrics["synced_lyrics"].is_null());
    let prep: Value =
        serde_json::from_slice(&fs::read(output.join("preparation.json")).unwrap()).unwrap();
    assert_eq!(prep["reference_delay_ms"], 25.);
    assert_eq!(prep["generated_delay_ms"], 0);
}
#[test]
fn source_and_original_role_corruption_fail_without_publishing() {
    for name in ["source.wav", "bass.wav"] {
        let temp = tempfile::tempdir().unwrap();
        let case = temp.path().join("case");
        fixture(&case);
        fs::write(case.join(name), b"corrupt").unwrap();
        let output = temp.path().join("oracle");
        assert!(prepare(&case, &output).is_err());
        assert!(!output.exists());
    }
}
#[test]
fn a_shorter_canonical_source_is_rejected_even_when_hash_witness_is_updated() {
    let temp = tempfile::tempdir().unwrap();
    let case = temp.path().join("case");
    fixture(&case);
    wave(&case.join("source.wav"), 900);
    mutate(&case.join("source.mix.json"), |m| {
        m["output_sha256"] = json!(hash_file(&case.join("source.wav")).unwrap())
    });
    let output = temp.path().join("oracle");
    assert!(prepare(&case, &output).is_err());
    assert!(!output.exists());
}
#[test]
fn decoded_role_clock_mismatch_is_rejected_not_trimmed() {
    let temp = tempfile::tempdir().unwrap();
    let case = temp.path().join("case");
    fixture(&case);
    mutate(&case.join("source.mix.json"), |m| {
        m["included_stems"][0]["frames"] = json!(500)
    });
    let output = temp.path().join("oracle");
    assert!(prepare(&case, &output).is_err());
    assert!(!output.exists());
}
#[test]
fn existing_output_and_changed_delay_witness_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let case = temp.path().join("case");
    fixture(&case);
    let output = temp.path().join("oracle");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"preserve").unwrap();
    assert!(prepare(&case, &output).is_err());
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"preserve");
    let fresh = temp.path().join("fresh");
    mutate(&case.join("source.mix.json"), |m| {
        m["ini_delay_milliseconds"] = json!(0)
    });
    assert!(prepare(&case, &fresh).is_err());
    assert!(!fresh.exists());
}

#[test]
fn evaluation_rejects_changed_artifacts_source_or_executable_and_retains_every_failure() {
    let temp = tempfile::tempdir().unwrap();
    let batch = temp.path().join("batch");
    let runs = temp.path().join("runs");
    fs::create_dir(&batch).unwrap();
    fs::create_dir_all(runs.join("heldout-01")).unwrap();
    let case = batch.join("heldout-01");
    fixture(&case);
    fs::write(
        batch.join("frozen-batch.json"),
        serde_json::to_vec(&json!({"songs":[{"archive_sha256":"a".repeat(64)}]})).unwrap(),
    )
    .unwrap();
    let generated = runs.join("heldout-01/generated.mid");
    fs::copy(case.join("notes.mid"), &generated).unwrap();
    let receipt = json!({"complete": true, "expected_cases": 1, "condition": "synthetic",
        "frozen_batch_sha256": hash_file(&batch.join("frozen-batch.json")).unwrap()});
    fs::write(
        runs.join("batch-run.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let valid = json!({"success": true, "engine_unchanged_after_run": true,
        "source_sha256": hash_file(&case.join("source.wav")).unwrap(),
        "source_archive_sha256": "a".repeat(64),
        "artifact": generated.canonicalize().unwrap(),
        "artifact_sha256": hash_file(&generated).unwrap()});
    for (index, field) in [
        None,
        Some("artifact_sha256"),
        Some("source_archive_sha256"),
        Some("engine_unchanged_after_run"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut run = valid.clone();
        if let Some(field) = field {
            run[field] = if field == "engine_unchanged_after_run" {
                json!(false)
            } else {
                json!("b".repeat(64))
            };
        }
        fs::write(
            runs.join("heldout-01/run.json"),
            serde_json::to_vec(&run).unwrap(),
        )
        .unwrap();
        let output = temp.path().join(format!("evaluation-{index}"));
        chart_benchmark::evaluate::evaluate(&batch, &runs, &output).unwrap();
        let case_report: Value =
            serde_json::from_slice(&fs::read(output.join("heldout-01.json")).unwrap()).unwrap();
        let results = case_report["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        if field.is_none() {
            assert!(
                results
                    .iter()
                    .any(|r| r["instrument"] == "vocals" && r["success"] == true)
            );
        } else {
            assert!(results.iter().all(|r| {
                r["success"] == false
                    && r["error"]
                        .as_str()
                        .unwrap()
                        .contains("Generation provenance invalid")
            }));
            let summary: Value =
                serde_json::from_slice(&fs::read(output.join("summary.json")).unwrap()).unwrap();
            assert!(summary["instruments"].as_array().unwrap().iter().all(|r| r["scored_cases"] == 0 && r["failures"].as_array().unwrap().len() == 1));
        }
        assert!(chart_benchmark::evaluate::evaluate(&batch, &runs, &output).is_err());
    }
}

#[test]
fn swapping_self_consistent_prepared_cases_cannot_bypass_frozen_archive_order() {
    let temp = tempfile::tempdir().unwrap();
    let batch = temp.path().join("batch");
    let runs = temp.path().join("runs");
    fs::create_dir(&batch).unwrap();
    fs::create_dir(&runs).unwrap();
    for index in 1..=2 {
        let case = batch.join(format!("heldout-{index:02}"));
        fixture(&case);
        let archive = if index == 1 { "a" } else { "b" }.repeat(64);
        mutate(&case.join("reference-source.json"), |v| {
            v["source_archive_sha256"] = json!(archive)
        });
        mutate(&case.join("source.mix.json"), |v| {
            v["source_archive_sha256"] = json!(archive)
        });
    }
    fs::write(
        batch.join("frozen-batch.json"),
        serde_json::to_vec(&json!({"songs":[
        {"archive_sha256":"a".repeat(64)},{"archive_sha256":"b".repeat(64)}]}))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        runs.join("batch-run.json"),
        serde_json::to_vec(
            &json!({"complete":true,"expected_cases":2,"condition":"synthetic",
        "frozen_batch_sha256":hash_file(&batch.join("frozen-batch.json")).unwrap()}),
        )
        .unwrap(),
    )
    .unwrap();
    for swapped in [false, true] {
        if swapped {
            let temporary = batch.join("swap");
            fs::rename(batch.join("heldout-01"), &temporary).unwrap();
            fs::rename(batch.join("heldout-02"), batch.join("heldout-01")).unwrap();
            fs::rename(temporary, batch.join("heldout-02")).unwrap();
        }
        for index in 1..=2 {
            let label = format!("heldout-{index:02}");
            let case = batch.join(&label);
            let output = runs.join(&label);
            fs::create_dir_all(&output).unwrap();
            let artifact = output.join("generated.mid");
            fs::copy(case.join("notes.mid"), &artifact).unwrap();
            let mix: Value =
                serde_json::from_slice(&fs::read(case.join("source.mix.json")).unwrap()).unwrap();
            fs::write(output.join("run.json"),serde_json::to_vec(&json!({"success":true,"engine_unchanged_after_run":true,
                "source_sha256":mix["output_sha256"],"source_archive_sha256":mix["source_archive_sha256"],
                "artifact":artifact.canonicalize().unwrap(),"artifact_sha256":hash_file(&artifact).unwrap()})).unwrap()).unwrap();
        }
        let evaluation = temp
            .path()
            .join(if swapped { "swapped" } else { "ordered" });
        chart_benchmark::evaluate::evaluate(&batch, &runs, &evaluation).unwrap();
        for index in 1..=2 {
            let report: Value = serde_json::from_slice(
                &fs::read(evaluation.join(format!("heldout-{index:02}.json"))).unwrap(),
            )
            .unwrap();
            let outcomes = report["results"].as_array().unwrap();
            if swapped {
                assert!(outcomes.iter().all(|r| {
                    r["success"] == false
                        && r["error"]
                            .as_str()
                            .unwrap()
                            .contains("differs from frozen selection")
                }));
            } else {
                assert!(
                    outcomes
                        .iter()
                        .any(|r| r["instrument"] == "vocals" && r["success"] == true)
                );
            }
        }
    }
}

#[test]
fn batch_refuses_swapped_prepared_archives_before_starting_inference() {
    let temp = tempfile::tempdir().unwrap();
    let batch = temp.path().join("batch");
    fs::create_dir(&batch).unwrap();
    let models = temp.path().join("models");
    fs::create_dir(&models).unwrap();
    let archives: [String; 6] = std::array::from_fn(|i| format!("{:x}", i + 1).repeat(64));
    for (i, archive) in archives.iter().enumerate() {
        let case = batch.join(format!("heldout-{:02}", i + 1));
        fixture(&case);
        mutate(&case.join("reference-source.json"), |v| {
            v["source_archive_sha256"] = json!(archive)
        });
        mutate(&case.join("source.mix.json"), |v| {
            v["source_archive_sha256"] = json!(archive)
        });
    }
    fs::write(
        batch.join("frozen-batch.json"),
        serde_json::to_vec(&json!({"songs":archives.iter()
        .map(|archive|json!({"archive_sha256":archive})).collect::<Vec<_>>()}))
        .unwrap(),
    )
    .unwrap();
    for pair in 0..3 {
        let a = batch.join(format!("heldout-{:02}", pair * 2 + 1));
        let b = batch.join(format!("heldout-{:02}", pair * 2 + 2));
        let temporary = batch.join("swap");
        fs::rename(&a, &temporary).unwrap();
        fs::rename(&b, &a).unwrap();
        fs::rename(temporary, &b).unwrap();
    }
    // Deliberately non-executable: no inference is authorized after a binding
    // failure. Its identity is not accessed by the rejected per-case jobs.
    let unused_engine = batch.join("heldout-01/notes.mid");
    let original = fs::read(&unused_engine).unwrap();
    let output = temp.path().join("out");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_chart-benchmark"))
        .arg("batch")
        .arg(&unused_engine)
        .arg(&batch)
        .arg(&output)
        .arg(&models)
        .arg("oracle")
        .output()
        .unwrap();
    assert!(!status.status.success());
    let receipt: Value =
        serde_json::from_slice(&fs::read(output.join("batch-run.json")).unwrap()).unwrap();
    assert_eq!(receipt["complete"], true);
    assert_eq!(receipt["outcomes"].as_array().unwrap().len(), 6);
    assert!(receipt["outcomes"].as_array().unwrap().iter().all(|o| {
        o["success"] == false
            && o["error"]
                .as_str()
                .unwrap()
                .contains("differs from frozen selection")
    }));
    for i in 1..=6 {
        assert!(!output.join(format!("heldout-{i:02}/run.json")).exists());
    }
    assert_eq!(fs::read(unused_engine).unwrap(), original);
}
