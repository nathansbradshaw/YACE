use std::{fs, process::Command};

#[test]
fn help_does_not_require_models_or_a_runtime() {
    let output = Command::new(env!("CARGO_BIN_EXE_yace-vocal-speech-probe"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("no lyrics, chart edits or model downloads"));
}

#[test]
fn incorrect_model_is_rejected_before_loading_a_runtime() {
    let root = tempfile::tempdir().unwrap();
    let model = root.path().join("wrong.onnx");
    fs::write(&model, b"not the pinned activity model").unwrap();
    let output_file = root.path().join("evidence.json");
    let output = Command::new(env!("CARGO_BIN_EXE_yace-vocal-speech-probe"))
        .arg("--input")
        .arg(root.path().join("missing.wav"))
        .arg("--model-file")
        .arg(model)
        .arg("--runtime")
        .arg(root.path().join("missing-runtime"))
        .arg("--output")
        .arg(&output_file)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum differs"));
    assert!(!output_file.exists());
}

#[test]
fn existing_evidence_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let output_file = root.path().join("evidence.json");
    fs::write(&output_file, b"keep").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yace-vocal-speech-probe"))
        .args(["--input", "missing.wav", "--model-file", "missing.onnx"])
        .arg("--output")
        .arg(&output_file)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(output_file).unwrap(), b"keep");
}
