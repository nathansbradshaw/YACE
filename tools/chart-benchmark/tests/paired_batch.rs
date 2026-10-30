use chart_engine::{
    chart::{self, Note, Tracks},
    mapping::{DrumHit, PitchNote, Transcription, Voice},
    models::hash_file,
    timing::TempoMap,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn save(path: &Path, v: &Value) {
    fs::write(path, serde_json::to_vec_pretty(v).unwrap()).unwrap();
}
fn load(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn wave(path: &Path) {
    let frames = 88200u32;
    let size = frames * 8;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((size + 36).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(44100u32.to_le_bytes());
    b.extend(352800u32.to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for _ in 0..frames {
        b.extend(0.1f32.to_le_bytes());
        b.extend(0.1f32.to_le_bytes());
    }
    fs::write(path, b).unwrap();
}
struct Fixture {
    root: tempfile::TempDir,
    prepared: PathBuf,
    baseline: PathBuf,
    candidate: PathBuf,
    template: PathBuf,
    models: PathBuf,
    calls: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = root.path();
        let baseline = base.join("baseline-audit");
        let candidate = base.join("candidate-audit");
        for (side, binary) in [("baseline", &baseline), ("candidate", &candidate)] {
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .args([
                        "--edition=2024",
                        "--crate-name",
                        "synthetic_paired_audit",
                        "--cfg",
                        &format!("{side}_audit")
                    ])
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures/paired_audit.rs")
                    )
                    .arg("-o")
                    .arg(binary)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert_ne!(
            hash_file(&baseline).unwrap(),
            hash_file(&candidate).unwrap()
        );
        let template = base.join("template");
        fs::create_dir(&template).unwrap();
        let timing = TempoMap::constant(120.).unwrap();
        for variant in ["baseline", "candidate", "moved-hand", "missing-guitar"] {
            let mut tracks = Tracks::new();
            let candidate = variant != "baseline";
            if variant != "missing-guitar" {
                let mut notes = vec![Note {
                    tick: 192,
                    lane: 0,
                    sustain: if candidate { 96 } else { 192 },
                }];
                if candidate {
                    notes.push(Note {
                        tick: 384,
                        lane: 1,
                        sustain: 0,
                    });
                }
                tracks.insert("ExpertSingle".into(), notes);
            }
            tracks.insert(
                "ExpertDoubleBass".into(),
                vec![Note {
                    tick: 192,
                    lane: 0,
                    sustain: 0,
                }],
            );
            let hand = if candidate { 3 } else { 2 };
            let tick = if variant == "moved-hand" { 384 } else { 192 };
            tracks.insert(
                "ExpertDrums".into(),
                vec![
                    Note {
                        tick: 192,
                        lane: 0,
                        sustain: 0,
                    },
                    Note {
                        tick,
                        lane: hand,
                        sustain: 0,
                    },
                    Note {
                        tick,
                        lane: 64 + hand,
                        sustain: 0,
                    },
                ],
            );
            fs::write(
                template.join(format!("{variant}.chart")),
                chart::new_chart(
                    "PRIVATE fixture title",
                    "PRIVATE fixture artist",
                    &timing,
                    &tracks,
                ),
            )
            .unwrap();
        }
        for variant in ["baseline", "candidate", "bad-bass", "empty-guitar"] {
            let mut events = Transcription {
                guitar: vec![PitchNote {
                    start: 0.5,
                    end: 1.,
                    pitch: 64,
                    confidence: 0.9,
                }],
                bass: vec![PitchNote {
                    start: 0.5,
                    end: 0.75,
                    pitch: 40,
                    confidence: 0.9,
                }],
                drums: vec![
                    DrumHit {
                        time: 0.5,
                        voice: Voice::Kick,
                        confidence: 0.9,
                    },
                    DrumHit {
                        time: 0.5,
                        voice: Voice::Tom,
                        confidence: 0.9,
                    },
                ],
                ..Default::default()
            };
            if variant != "baseline" {
                events.guitar[0].end = 0.75;
                events.guitar.push(PitchNote {
                    start: 1.,
                    end: 1.2,
                    pitch: 67,
                    confidence: 0.9,
                });
            }
            if variant == "bad-bass" {
                events.bass[0].pitch = 41;
            }
            if variant == "empty-guitar" {
                events.guitar.clear();
            }
            save(
                &template.join(format!("{variant}-events.json")),
                &serde_json::to_value(events).unwrap(),
            );
        }
        for side in ["baseline", "candidate"] {
            let mut events: Transcription =
                serde_json::from_value(load(&template.join("baseline-events.json"))).unwrap();
            events.drums = vec![
                DrumHit {
                    time: 0.5,
                    voice: Voice::Snare,
                    confidence: 0.95,
                },
                DrumHit {
                    time: 0.5,
                    voice: Voice::Tom,
                    confidence: 0.60,
                },
                DrumHit {
                    time: 0.5,
                    voice: if side == "baseline" {
                        Voice::Cymbal
                    } else {
                        Voice::Ride
                    },
                    confidence: 0.80,
                },
            ];
            let variant = format!("survivor-{side}");
            let tracks = chart_engine::mapping::tracks(&events, &timing, true).unwrap();
            fs::write(
                template.join(format!("{variant}.chart")),
                chart::new_chart("Synthetic source survival", "Fixture", &timing, &tracks),
            )
            .unwrap();
            save(
                &template.join(format!("{variant}-events.json")),
                &serde_json::to_value(events).unwrap(),
            );
        }
        let prepared = base.join("prepared");
        fs::create_dir(&prepared).unwrap();
        save(
            &prepared.join("frozen-batch.json"),
            &json!({"songs":(1..=6).map(|i|json!({"archive_sha256":format!("{i:064x}")})).collect::<Vec<_>>()}),
        );
        for index in 1..=6 {
            let case = prepared.join(format!("heldout-{index:02}"));
            fs::create_dir(&case).unwrap();
            wave(&case.join("source.wav"));
            for role in ["bass", "guitar", "drums", "vocals"] {
                fs::copy(case.join("source.wav"), case.join(format!("{role}.wav"))).unwrap();
            }
            fs::copy(template.join("candidate.chart"), case.join("notes.chart")).unwrap();
            fs::write(case.join("song.ini"), "[song]\ndelay = 0\n").unwrap();
            let assets:Vec<_>=["bass.wav","guitar.wav","drums.wav","vocals.wav","notes.chart"].into_iter().map(|name|json!({"name":name,"bytes":fs::metadata(case.join(name)).unwrap().len(),"sha256":hash_file(&case.join(name)).unwrap()})).collect();
            let archive = format!("{index:064x}");
            save(
                &case.join("reference-source.json"),
                &json!({"version":1,"assets":assets,"metadata":{"delay":"0"},"song_ini_sha256":hash_file(&case.join("song.ini")).unwrap(),"source_archive_sha256":archive}),
            );
            let stems:Vec<_>=["bass","guitar","drums","vocals"].into_iter().map(|role|json!({"stem":role,"sha256":hash_file(&case.join(format!("{role}.wav"))).unwrap(),"frames":88200,"starts_at_source_frame":0})).collect();
            save(
                &case.join("source.mix.json"),
                &json!({"version":1,"output_sha256":hash_file(&case.join("source.wav")).unwrap(),"source_archive_sha256":archive,"sample_rate":44100,"channels":2,"frames":88200,"duration_seconds":2.,"ini_delay_milliseconds":0.,"included_stems":stems}),
            );
        }
        let oracle = base.join("oracle-witness");
        chart_benchmark::prepare(&prepared.join("heldout-01"), &oracle).unwrap();
        let comparison = json!({"version":1,"audio_sha256":hash_file(&prepared.join("heldout-01/source.wav")).unwrap(),"stem_manifest_sha256":hash_file(&oracle.join("stem-manifest.json")).unwrap()});
        save(&template.join("comparison.json"), &comparison);
        let mut wrong = comparison;
        wrong["audio_sha256"] = json!("f".repeat(64));
        save(&template.join("wrong-source.json"), &wrong);
        let mut omitted = load(&template.join("candidate-events.json"));
        omitted.as_object_mut().unwrap().remove("drums");
        save(&template.join("omitted-drums-events.json"), &omitted);
        let models = base.join("models");
        fs::create_dir(&models).unwrap();
        for name in ["basic_pitch.onnx", "adtof_frame_rnn.onnx"] {
            fs::write(
                models.join(name),
                b"synthetic identity only; never executed",
            )
            .unwrap();
        }
        let calls = base.join("calls.log");
        Self {
            root,
            prepared,
            baseline,
            candidate,
            template,
            models,
            calls,
        }
    }
    fn invoke(&self, mode: &str, output: &Path, skipped: Option<&str>) -> std::process::Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_chart-benchmark"));
        c.arg("paired-instrument-batch")
            .arg(&self.baseline)
            .arg(&self.candidate)
            .arg(&self.prepared)
            .arg(output)
            .arg(&self.models)
            .env("FAKE_PAIR_TEMPLATE", &self.template)
            .env("FAKE_PAIR_CALLS", &self.calls)
            .env("FAKE_PAIR_MODE", mode);
        if let Some(label) = skipped {
            c.env("FAKE_PAIR_PREP_FAILED", label);
        }
        c.output().unwrap()
    }
}
#[test]
fn distinct_policies_use_revised_only_and_retain_independent_failures_and_invariants() {
    let f = Fixture::new();
    let output = f.root.path().join("mixed");
    let result = f.invoke("failures", &output, None);
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let v = load(&output.join("summary.json"));
    assert_eq!(v["complete"], true);
    assert_eq!(v["all_cases_successful"], false);
    assert_eq!(v["cases"].as_array().unwrap().len(), 6);
    let calls = fs::read_to_string(&f.calls).unwrap();
    assert_eq!(calls.lines().count(), 12);
    assert_eq!(
        calls.lines().filter(|s| s.starts_with("baseline:")).count(),
        6
    );
    assert_eq!(
        calls
            .lines()
            .filter(|s| s.starts_with("candidate:"))
            .count(),
        6
    );
    assert_eq!(v["cases"][0]["invariants"]["success"], true);
    assert_eq!(
        v["cases"][0]["invariants"]["guitar_mapping"]["added_heads"],
        1
    );
    assert_eq!(
        v["cases"][0]["invariants"]["guitar_mapping"]["changed_sustains_on_existing_heads"],
        1
    );
    assert_eq!(
        v["cases"][0]["invariants"]["drum_mapping"]["added_nonplayed_marker_triples"],
        1
    );
    assert_eq!(v["cases"][1]["baseline"]["success"], false);
    assert_eq!(v["cases"][1]["candidate"]["success"], true);
    assert_eq!(v["cases"][2]["candidate"]["success"], false);
    assert_eq!(
        v["cases"][3]["invariants"]["bass_raw_and_mapped_unchanged"],
        false
    );
    assert_eq!(
        v["cases"][4]["invariants"]["mapped_kick_hand_attack_tick_sets_unchanged"],
        false
    );
    for row in v["instruments"].as_array().unwrap() {
        assert_eq!(row["baseline"]["scored_cases"], 5);
        assert_eq!(
            row["candidate"]["scored_cases"],
            if row["instrument"] == "guitar" { 4 } else { 5 }
        );
        assert_eq!(
            row["paired_cases"],
            if row["instrument"] == "guitar" { 1 } else { 2 }
        );
        assert_eq!(
            row["paired_metrics"]["baseline"]["attacks"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    let freeze = load(&output.join("freeze.json"));
    assert_eq!(
        freeze["plan_sha256"],
        hash_file(&output.join("plan.json")).unwrap()
    );
    assert_ne!(
        freeze["baseline_audit_sha256"],
        freeze["candidate_audit_sha256"]
    );
    assert_eq!(
        freeze["all_case_preparation_finished_before_inference"],
        true
    );
    let public = serde_json::to_string(&v).unwrap();
    assert!(!public.contains("PRIVATE") && !public.contains(f.root.path().to_str().unwrap()));
    let bytes = fs::read(output.join("summary.json")).unwrap();
    assert!(!f.invoke("clean", &output, None).status.success());
    assert_eq!(fs::read(output.join("summary.json")).unwrap(), bytes);
    assert_eq!(fs::read_to_string(&f.calls).unwrap().lines().count(), 12);
}
#[test]
fn frozen_archive_substitution_skips_both_runs_but_keeps_the_case() {
    let f = Fixture::new();
    let folder = f.prepared.join("heldout-03");
    for name in ["reference-source.json", "source.mix.json"] {
        let path = folder.join(name);
        let mut v = load(&path);
        v["source_archive_sha256"] = json!(format!("{:064x}", 4));
        save(&path, &v);
    }
    let out = f.root.path().join("substitution");
    assert!(!f.invoke("clean", &out, Some("heldout-03")).status.success());
    let v = load(&out.join("summary.json"));
    assert_eq!(v["cases"][2]["preparation_success"], false);
    assert_eq!(fs::read_to_string(&f.calls).unwrap().lines().count(), 10);
    for instrument in v["instruments"].as_array().unwrap() {
        assert_eq!(instrument["paired_cases"], 5);
    }
    assert_eq!(
        load(&out.join("plan.json"))["cases"][2]["preparation_success"],
        false
    );
}
#[test]
fn empty_raw_and_missing_part_is_explicit_abstention_and_plan_mismatch_fails_before_inference() {
    let f = Fixture::new();
    let invalid_model = f.models.join("oaf_drums.onnx");
    fs::write(&invalid_model, b"unverified model must never enter audit").unwrap();
    let invalid_output = f.root.path().join("invalid-model");
    let rejected = f.invoke("empty", &invalid_output, None);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("OAF model"));
    assert!(!invalid_output.exists() && !f.calls.exists());
    fs::remove_file(invalid_model).unwrap();
    let plan = f.root.path().join("paired-instrument-followup-plan.json");
    save(&plan, &json!({"selection_sha256":"f".repeat(64)}));
    let out = f.root.path().join("good");
    assert!(!f.invoke("empty", &out, None).status.success());
    assert!(!out.exists() && !f.calls.exists());
    let ambiguous = f.root.path().join("recovery-followup-plan.json");
    save(
        &ambiguous,
        &json!({"selection_sha256":hash_file(&f.prepared.join("frozen-batch.json")).unwrap()}),
    );
    assert!(!f.invoke("empty", &out, None).status.success());
    assert!(!out.exists() && !f.calls.exists());
    fs::remove_file(ambiguous).unwrap();
    for key in ["baseline_audit_sha256", "candidate_audit_sha256"] {
        let mut declared = json!({"selection_sha256":hash_file(&f.prepared.join("frozen-batch.json")).unwrap(),"baseline_audit_sha256":hash_file(&f.baseline).unwrap(),"candidate_audit_sha256":hash_file(&f.candidate).unwrap()});
        declared[key] = json!("f".repeat(64));
        save(&plan, &declared);
        let rejected = f.invoke("empty", &out, None);
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr).contains(key));
        assert!(!out.exists() && !f.calls.exists());
    }
    save(
        &plan,
        &json!({"selection_sha256":hash_file(&f.prepared.join("frozen-batch.json")).unwrap()}),
    );
    let result = f.invoke("empty", &out, None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let v = load(&out.join("summary.json"));
    assert_eq!(v["all_cases_successful"], true);
    let guitar = &v["cases"][5]["results"][0];
    assert_eq!(guitar["candidate"]["success"], true);
    assert_eq!(guitar["candidate"]["abstained"], true);
    let freeze = load(&out.join("freeze.json"));
    assert_eq!(
        freeze["external_declaration_plan_sha256"],
        hash_file(&plan).unwrap()
    );
    let published_plan = load(&out.join("plan.json"));
    for key in ["engine-midi.rs", "engine-Cargo.toml", "engine-Cargo.lock"] {
        assert_eq!(
            published_plan["freeze"]["runner_source_files"][key],
            hash_file(&out.join("runner-source").join(key)).unwrap()
        );
    }
    assert_eq!(fs::read_to_string(&f.calls).unwrap().lines().count(), 12);
}

#[test]
fn omitted_raw_arrays_and_late_sibling_artifact_substitution_are_retained_failures() {
    let f = Fixture::new();
    let out = f.root.path().join("artifact-ambiguity");
    assert!(!f.invoke("artifact-ambiguity", &out, None).status.success());
    let v = load(&out.join("summary.json"));
    assert_eq!(v["cases"][0]["baseline"]["execution_success"], true);
    assert_eq!(v["cases"][0]["invariants"]["success"], false);
    assert_eq!(
        v["cases"][0]["invariants"]["failure_code"],
        "final_pair_validation_failed"
    );
    assert_eq!(v["cases"][2]["candidate"]["execution_success"], false);
    let original = load(&out.join("heldout-01/execution-case.json"));
    assert_eq!(original["invariants"]["success"], true);
    assert_eq!(
        load(&out.join("heldout-01/case.json"))["invariants"]["success"],
        false
    );
    assert_eq!(fs::read_to_string(&f.calls).unwrap().lines().count(), 12);
    for instrument in v["instruments"].as_array().unwrap() {
        assert_eq!(instrument["paired_cases"], 4);
    }
}

#[test]
fn unchanged_timing_and_headcounts_cannot_hide_a_different_selected_drum_source() {
    let f = Fixture::new();
    let out = f.root.path().join("source-survivor");
    let result = f.invoke("source-survivor", &out, None);
    assert!(!result.status.success());
    let v = load(&out.join("summary.json"));
    let invariants = &v["cases"][0]["invariants"];
    assert_eq!(
        invariants["raw_drums"]["kick_hand_time_multisets_unchanged"],
        true
    );
    assert_eq!(
        invariants["raw_drums"]["confidence_and_voice_rules_preserved"],
        true
    );
    assert_eq!(
        invariants["mapped_kick_hand_attack_tick_sets_unchanged"],
        true
    );
    assert_eq!(invariants["baseline_selected_hand_sources"], 2);
    assert_eq!(invariants["candidate_selected_hand_sources"], 2);
    assert_eq!(invariants["selected_raw_drum_sources_unchanged"], false);
    assert_eq!(invariants["success"], false);
    for instrument in v["instruments"].as_array().unwrap() {
        assert_eq!(instrument["baseline"]["scored_cases"], 6);
        assert_eq!(instrument["candidate"]["scored_cases"], 6);
        assert_eq!(instrument["paired_cases"], 5);
    }
    assert_eq!(fs::read_to_string(&f.calls).unwrap().lines().count(), 12);
}
