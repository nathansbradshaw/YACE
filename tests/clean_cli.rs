use chart_engine::{
    clean::{AlignedWord, CleanPlan, CleanPolicy, CleanReceipt, StemAlignment, WordTimings},
    models,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const FRAMES: usize = 4410;

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chart-engine"));
    command.current_dir(root).arg("clean");
    command
}

fn success(result: Output) -> serde_json::Value {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn policy() -> CleanPolicy {
    CleanPolicy {
        version: 1,
        name: "Explicit test choices".into(),
        terms: vec!["heck".into(), "ass".into()],
        padding_before_seconds: 0.,
        padding_after_seconds: 0.,
        fade_seconds: 0.,
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn make_plan(root: &Path, words: &Path, policy: &Path, mode: &str, output: &Path) -> Output {
    command(root)
        .args(["plan", "--words"])
        .arg(words)
        .arg("--policy")
        .arg(policy)
        .args(["--mode", mode, "--output"])
        .arg(output)
        .output()
        .unwrap()
}

fn render(
    root: &Path,
    audio: &Path,
    plan: &Path,
    stem: Option<&Path>,
    alignment: Option<&Path>,
    output: &Path,
) -> Output {
    let mut command = command(root);
    command
        .args(["render", "--audio"])
        .arg(audio)
        .arg("--plan")
        .arg(plan)
        .arg("--output")
        .arg(output);
    if let Some(stem) = stem {
        command.arg("--vocal-stem").arg(stem);
    }
    if let Some(alignment) = alignment {
        command.arg("--alignment").arg(alignment);
    }
    command.output().unwrap()
}

#[test]
fn cli_defaults_to_common_explicit_markers_and_custom_policy_replaces_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let words_path = root.join("words.json");
    let words = WordTimings {
        version: 1,
        audio_sha256: "a".repeat(64),
        duration_seconds: 3.,
        source: "Manually aligned policy-selection fixture".into(),
        words: ["fuck", "heck", "class"]
            .iter()
            .enumerate()
            .map(|(index, text)| AlignedWord {
                text: Some((*text).into()),
                start: Some(index as f64),
                end: Some(index as f64 + 0.5),
            })
            .collect(),
    };
    write_json(&words_path, &words);
    let default_path = root.join("default-plan.json");
    let default = success(
        command(root)
            .args(["plan", "--words"])
            .arg(&words_path)
            .args(["--mode", "full-mix", "--output"])
            .arg(&default_path)
            .output()
            .unwrap(),
    );
    assert_eq!(
        default["policy"],
        serde_json::to_value(CleanPolicy::default()).unwrap()
    );
    assert_eq!(default["matches"].as_array().unwrap().len(), 1);
    assert_eq!(default["matches"][0]["term"], "fuck");

    let policy_path = root.join("custom-policy.json");
    write_json(&policy_path, &policy());
    let custom = success(make_plan(
        root,
        &words_path,
        &policy_path,
        "full-mix",
        &root.join("custom-plan.json"),
    ));
    assert_eq!(custom["policy"], serde_json::to_value(policy()).unwrap());
    assert_eq!(custom["matches"].as_array().unwrap().len(), 1);
    assert_eq!(custom["matches"][0]["term"], "heck");

    fs::write(&policy_path, b"invalid JSON").unwrap();
    let refused = root.join("invalid-policy-plan.json");
    assert!(
        !make_plan(root, &words_path, &policy_path, "full-mix", &refused)
            .status
            .success()
    );
    assert!(!refused.exists());
}

#[test]
fn full_mix_cli_retains_unknowns_and_refuses_stale_audio_or_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let audio = root.join("original.wav");
    models::write_audio(&audio, &vec![0.6; FRAMES], &vec![-0.3; FRAMES]).unwrap();
    let original_bytes = fs::read(&audio).unwrap();
    let words = WordTimings {
        version: 1,
        audio_sha256: models::hash_file(&audio).unwrap(),
        duration_seconds: 0.1,
        source: "Original synthetic test words, manually timed; no ASR".into(),
        words: vec![
            AlignedWord {
                text: Some("HECK!".into()),
                start: Some(0.02),
                end: Some(0.04),
            },
            AlignedWord {
                text: None,
                start: Some(0.05),
                end: Some(0.06),
            },
            AlignedWord {
                text: Some("class".into()),
                start: Some(0.07),
                end: Some(0.08),
            },
            AlignedWord {
                text: Some("heck".into()),
                start: None,
                end: None,
            },
        ],
    };
    let words_path = root.join("words.json");
    let policy_path = root.join("policy.json");
    let plan_path = root.join("plan.json");
    write_json(&words_path, &words);
    write_json(&policy_path, &policy());
    let result = success(make_plan(
        root,
        &words_path,
        &policy_path,
        "full-mix",
        &plan_path,
    ));
    assert_eq!(result["unknown_words"], 1);
    assert_eq!(result["untimed_words"], 1);
    let plan: CleanPlan = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    assert_eq!(plan.matches.len(), 2);
    assert_eq!(plan.ranges.len(), 1);
    assert_eq!(plan.matches[1].start, None);
    let plan_bytes = fs::read(&plan_path).unwrap();
    assert!(
        !make_plan(root, &words_path, &policy_path, "full-mix", &plan_path)
            .status
            .success()
    );
    assert_eq!(fs::read(&plan_path).unwrap(), plan_bytes);

    let output = root.join("clean-copy");
    let result = success(render(root, &audio, &plan_path, None, None, &output));
    assert_eq!(result["mode"], "full_mix");
    assert_eq!(result["channels"], 2);
    assert_eq!(result["unresolved_matches"], 1);
    let receipt: CleanReceipt =
        serde_json::from_slice(&fs::read(output.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt.source_sha256, words.audio_sha256);
    assert_eq!(receipt.frames, FRAMES);
    let samples = models::decode(&output.join("clean.wav"), 44100, 2).unwrap();
    assert_eq!(samples.len(), FRAMES * 2);
    for (frame, channels) in samples.chunks_exact(2).enumerate() {
        if (882..1764).contains(&frame) {
            assert!(channels.iter().all(|sample| sample.abs() < 1e-7));
        } else {
            assert!((channels[0] - 0.6).abs() < 1e-6);
            assert!((channels[1] + 0.3).abs() < 1e-6);
        }
    }
    let exported = fs::read(output.join("clean.wav")).unwrap();
    let receipt_bytes = fs::read(output.join("receipt.json")).unwrap();
    assert!(
        !render(root, &audio, &plan_path, None, None, &output)
            .status
            .success()
    );
    assert_eq!(fs::read(output.join("clean.wav")).unwrap(), exported);
    assert_eq!(
        fs::read(output.join("receipt.json")).unwrap(),
        receipt_bytes
    );

    let other = root.join("other-recording.wav");
    models::write_audio(&other, &vec![0.7; FRAMES], &vec![-0.3; FRAMES]).unwrap();
    let refused = root.join("stale-audio");
    let result = render(root, &other, &plan_path, None, None, &refused);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Stale clean plan"));
    assert!(!refused.exists());
    let refused = root.join("mode-mismatch");
    assert!(
        !render(root, &audio, &plan_path, Some(&audio), None, &refused)
            .status
            .success()
    );
    assert!(!refused.exists());
    assert_eq!(fs::read(&audio).unwrap(), original_bytes);
    assert!(!root.join("models").exists());
}

#[test]
fn vocal_only_cli_binds_stems_keeps_backing_and_never_falls_back() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let audio = root.join("original.wav");
    let stem = root.join("vocals.wav");
    models::write_audio(&audio, &vec![0.6; FRAMES], &vec![0.3; FRAMES]).unwrap();
    models::write_audio(&stem, &vec![0.2; FRAMES], &vec![0.1; FRAMES]).unwrap();
    let original_bytes = fs::read(&audio).unwrap();
    let original_stem = fs::read(&stem).unwrap();
    let words = WordTimings {
        version: 1,
        audio_sha256: models::hash_file(&audio).unwrap(),
        duration_seconds: 0.1,
        source: "Original synthetic sample-aligned additive fixture".into(),
        words: vec![AlignedWord {
            text: Some("heck".into()),
            start: Some(0.),
            end: Some(0.1),
        }],
    };
    let words_path = root.join("words.json");
    let policy_path = root.join("policy.json");
    let plan_path = root.join("plan.json");
    let binding = root.join("alignment.json");
    write_json(&words_path, &words);
    write_json(&policy_path, &policy());
    success(make_plan(
        root,
        &words_path,
        &policy_path,
        "vocal-only",
        &plan_path,
    ));
    let bind = |evidence: &str| {
        command(root)
            .args(["bind-stem", "--audio"])
            .arg(&audio)
            .arg("--stem")
            .arg(&stem)
            .args(["--evidence", evidence, "--output"])
            .arg(&binding)
            .output()
            .unwrap()
    };
    assert!(!bind("  ").status.success());
    assert!(!binding.exists());
    success(bind("Synthetic sample-aligned additive vocal stem"));
    let alignment: StemAlignment = serde_json::from_slice(&fs::read(&binding).unwrap()).unwrap();
    assert_eq!(alignment.audio_sha256, words.audio_sha256);
    assert_eq!(alignment.stem_sha256, models::hash_file(&stem).unwrap());
    let binding_bytes = fs::read(&binding).unwrap();
    assert!(
        !bind("Replacement assertion must not overwrite the existing one")
            .status
            .success()
    );
    assert_eq!(fs::read(&binding).unwrap(), binding_bytes);

    let refused = root.join("missing-stem");
    assert!(
        !render(root, &audio, &plan_path, None, Some(&binding), &refused)
            .status
            .success()
    );
    assert!(!refused.exists());
    let refused = root.join("missing-assertion");
    assert!(
        !render(root, &audio, &plan_path, Some(&stem), None, &refused)
            .status
            .success()
    );
    assert!(!refused.exists());
    let output = root.join("vocal-only-copy");
    let receipt = success(render(
        root,
        &audio,
        &plan_path,
        Some(&stem),
        Some(&binding),
        &output,
    ));
    assert_eq!(receipt["mode"], "vocal_only");
    assert_eq!(receipt["channels"], 2);
    assert_eq!(receipt["vocal_stem_sha256"], alignment.stem_sha256);
    let samples = models::decode(&output.join("clean.wav"), 44100, 2).unwrap();
    for channels in samples.chunks_exact(2) {
        assert!((channels[0] - 0.4).abs() < 1e-6);
        assert!((channels[1] - 0.2).abs() < 1e-6);
    }
    assert_eq!(fs::read(&audio).unwrap(), original_bytes);
    assert_eq!(fs::read(&stem).unwrap(), original_stem);

    let changed = root.join("changed-stem.wav");
    models::write_audio(&changed, &vec![0.3; FRAMES], &vec![0.1; FRAMES]).unwrap();
    // Simulate external replacement at the selected path after binding.
    let changed_bytes = fs::read(&changed).unwrap();
    fs::write(&stem, &changed_bytes).unwrap();
    let refused = root.join("stale-stem");
    let result = render(
        root,
        &audio,
        &plan_path,
        Some(&stem),
        Some(&binding),
        &refused,
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("alignment provenance"));
    assert!(!refused.exists());
    assert_eq!(fs::read(&stem).unwrap(), changed_bytes);
    assert_eq!(fs::read(&audio).unwrap(), original_bytes);
    assert!(!root.join("models").exists());
}
