use std::{fs, process::Command};

#[test]
fn existing_output_is_rejected_before_loading_a_model() {
    let folder = tempfile::tempdir().unwrap();
    let output = folder.path().join("existing");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("sentinel"), b"preserve").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yace-singing-pitch-probe"))
        .args([
            "--model",
            "tiny",
            "--model-file",
            "missing.onnx",
            "--controls",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Output already exists"));
    assert_eq!(fs::read(output.join("sentinel")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(output).unwrap().count(), 1);
}

#[test]
fn invalid_weights_do_not_create_output_or_invoke_decoder() {
    let folder = tempfile::tempdir().unwrap();
    let model = folder.path().join("wrong.onnx");
    let output = folder.path().join("not-created");
    fs::write(&model, b"incorrect weights").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yace-singing-pitch-probe"))
        .args(["--model", "tiny", "--model-file"])
        .arg(model)
        .args(["--input", "missing-audio.wav", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Pinned model checksum mismatch"));
    assert!(!output.exists());
}
