use std::{fs, process::Command};

#[test]
#[ignore = "requires YACE_LARSNET_MODELS with the five pinned official checkpoints"]
fn pinned_models_keep_their_exact_onnx_outputs() {
    let models = std::path::PathBuf::from(
        std::env::var_os("YACE_LARSNET_MODELS").expect("set YACE_LARSNET_MODELS"),
    );
    let root = tempfile::tempdir().unwrap();
    for (part, expected) in [
        (
            "kick",
            "15f1dcd32d883a7c76827d374a56e960b87679fddfe9e35527811fedeb3c3351",
        ),
        (
            "snare",
            "16b8d4c7882473f3cccfdfad36583aa0cdbcb0c782a2db3f0ed4b7b870bf8555",
        ),
        (
            "toms",
            "f8f279925383d539d3fef7ab43ed024be2ca483ad0e5e855484b045e31ea0e00",
        ),
        (
            "hihat",
            "972d6bfe012436156f9fc66dc062bc774fb620fcad3f895c35f2acedd5499b32",
        ),
        (
            "cymbals",
            "69110dd7705a09935590ef9a22dfecd5e8d1af5656890b2d3dbd764015f9a727",
        ),
    ] {
        let output = root.path().join(format!("{part}.onnx"));
        let result = Command::new(env!("CARGO_BIN_EXE_larsnet-convert"))
            .arg(part)
            .arg(models.join(format!("{part}.pt")))
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{part}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(output.with_extension("onnx.receipt.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["onnx_sha256"], expected, "{part}");
        assert_eq!(receipt["onnx_bytes"], 39_341_756);
    }
}

#[test]
fn legacy_candle_keeps_pinned_archive_values_with_zip_nine() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/candle-core-legacy/tests");
    let arrays = candle_core::Tensor::read_npz(fixtures.join("test.npz")).unwrap();
    assert_eq!(arrays[0].0, "x");
    assert_eq!(
        arrays[1]
            .1
            .to_dtype(candle_core::DType::U8)
            .unwrap()
            .to_vec1::<u8>()
            .unwrap(),
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    let tensors =
        candle_core::pickle::PthTensors::new(fixtures.join("fortran_tensor_3d.pth"), None).unwrap();
    let tensor = tensors.get("tensor_fortran").unwrap().unwrap();
    assert_eq!(tensor.dims3().unwrap(), (2, 3, 4));
    assert_eq!(
        tensor
            .to_vec3::<i64>()
            .unwrap()
            .into_iter()
            .flatten()
            .flatten()
            .collect::<Vec<_>>(),
        (1..=24).collect::<Vec<_>>()
    );
}

#[test]
fn unpinned_checkpoint_fails_before_model_parse_or_publication() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("untrusted.pt");
    let output = dir.path().join("kick.onnx");
    fs::write(&input, b"not an official checkpoint").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_larsnet-convert"))
        .arg("kick")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("checkpoint mismatch"));
    assert!(!output.exists());
    assert!(!output.with_extension("onnx.receipt.json").exists());
}

#[test]
fn existing_output_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("kick.onnx");
    fs::write(&output, b"keep existing data").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_larsnet-convert"))
        .arg("kick")
        .arg("missing.pt")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(output).unwrap(), b"keep existing data");
}
