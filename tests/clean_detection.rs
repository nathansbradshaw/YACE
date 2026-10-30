use chart_engine::{
    clean::{self, AlignedWord, CleanMode, CleanPolicy, WordTimings},
    clean_recognition::{self, Detection, Recognition},
    models,
};
use std::{fs, process::Command};

fn detection() -> Detection {
    let words = WordTimings {
        version: 1,
        audio_sha256: "a".repeat(64),
        duration_seconds: 1.,
        source: "Automatic test provenance".into(),
        words: vec![AlignedWord {
            text: Some("fuck".into()),
            start: Some(0.2),
            end: Some(0.5),
        }],
    };
    let plan = clean::plan(&words, &CleanPolicy::default(), CleanMode::FullMix).unwrap();
    Detection {
        recognition: Recognition {
            version: 1,
            backend: "Test boundary adapter".into(),
            model_sha256: "b".repeat(64),
            recognition_audio_sha256: words.audio_sha256.clone(),
            language: "en".into(),
            words,
            segments: vec![],
            warnings: vec![clean_recognition::RECOGNITION_WARNING.into()],
        },
        plan,
    }
}

#[test]
fn automatic_results_save_words_and_reviewable_plan_without_overwriting_history() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("analysis");
    let detection = detection();
    clean_recognition::save(&output, &detection).unwrap();
    let saved_words: WordTimings =
        serde_json::from_slice(&fs::read(output.join("words.json")).unwrap()).unwrap();
    assert_eq!(saved_words, detection.recognition.words);
    let saved_plan: clean::CleanPlan =
        serde_json::from_slice(&fs::read(output.join("plan.json")).unwrap()).unwrap();
    assert_eq!(saved_plan, detection.plan);
    let original = fs::read(output.join("recognition.json")).unwrap();
    assert!(clean_recognition::save(&output, &detection).is_err());
    assert_eq!(fs::read(output.join("recognition.json")).unwrap(), original);
    let first = clean_recognition::save_run(root.path(), &detection).unwrap();
    let second = clean_recognition::save_run(root.path(), &detection).unwrap();
    assert_ne!(first, second);
    assert!(first.join("words.json").is_file() && second.join("plan.json").is_file());
    let mut bad = detection;
    bad.plan.audio_sha256 = "c".repeat(64);
    let refused = root.path().join("wrong-source");
    assert!(clean_recognition::save(&refused, &bad).is_err());
    assert!(!refused.exists());
}

#[test]
fn detect_cli_refuses_missing_or_corrupt_model_without_publishing_results() {
    let root = tempfile::tempdir().unwrap();
    let audio = root.path().join("recording.wav");
    models::write_audio(&audio, &vec![0.1; 44100], &vec![0.1; 44100]).unwrap();
    let original = fs::read(&audio).unwrap();
    let model_dir = root.path().join("models");
    let output = root.path().join("analysis");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_chart-engine"))
            .args(["clean", "detect", "--audio"])
            .arg(&audio)
            .arg("--model-dir")
            .arg(&model_dir)
            .arg("--no-download")
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let missing = run();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("model is missing"));
    assert!(!model_dir.exists() && !output.exists());
    fs::create_dir(&model_dir).unwrap();
    fs::write(model_dir.join("ggml-small.en-q5_1.bin"), b"corrupt model").unwrap();
    let corrupt = run();
    assert!(!corrupt.status.success());
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("checksum mismatch"));
    assert!(!output.exists());
    assert_eq!(fs::read(audio).unwrap(), original);
}
