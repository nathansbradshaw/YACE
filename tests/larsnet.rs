use chart_engine::larsnet::{self, MODELS, RECEIPT_FILENAME};
use std::{fs, path::Path};

#[test]
fn optional_models_missing_or_corrupt_never_publish_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input.wav");
    fs::write(&source, b"unchanged source bytes").unwrap();
    let out = temp.path().join("result");
    assert!(larsnet::separate_files(&source, temp.path(), &out).is_err());
    assert!(!out.exists());
    fs::write(temp.path().join("kick.onnx"), b"not a pinned model").unwrap();
    assert!(larsnet::separate_files(&source, temp.path(), &out).is_err());
    assert!(!out.exists());
    assert_eq!(fs::read(source).unwrap(), b"unchanged source bytes");
}

#[test]
fn cancellation_and_existing_destination_are_non_destructive() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"keep me").unwrap();
    assert!(larsnet::separate_files(Path::new("missing"), temp.path(), &existing).is_err());
    assert_eq!(fs::read(existing.join("keep")).unwrap(), b"keep me");
    let out = temp.path().join("cancelled");
    let error =
        larsnet::separate_files_with_progress(Path::new("missing"), temp.path(), &out, |_, _| {
            anyhow::bail!("cancelled by caller")
        })
        .unwrap_err();
    assert!(error.to_string().contains("cancelled by caller"));
    assert!(!out.exists());
}

/// Supply only explicitly installed, checksum-pinned local CC-BY-NC4.0 models.
/// This checks native silence/clock/file contracts, not musical separation truth.
#[test]
#[ignore = "requires optional local LarsNet models and FFmpeg"]
fn native_silence_has_five_exact_length_silent_outputs_and_bound_receipt() {
    let model_dir = std::env::var_os("YACE_LARSNET_MODELS").expect("YACE_LARSNET_MODELS");
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("silence.wav");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=stereo",
            "-t",
            "0.125",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let out = temp.path().join("separated");
    let receipt = larsnet::separate_files(&source, Path::new(&model_dir), &out).unwrap();
    assert_eq!(receipt.sample_rate, 44100);
    assert_eq!(receipt.channels, 2);
    assert_eq!(receipt.models.len(), 5);
    let decoded = chart_engine::models::decode(&source, 44100, 2).unwrap();
    assert_eq!(receipt.frames, (decoded.len() / 2) as u64);
    assert_eq!(receipt.outputs.len(), 5);
    assert!(receipt.source_audio_sha256.is_some());
    for (output, (part, hash)) in receipt.outputs.iter().zip(MODELS) {
        assert_eq!(output.part, part);
        assert_eq!(receipt.models[part], hash);
        assert_eq!(output.frames, receipt.frames);
        assert_eq!(output.rms, 0.);
        assert_eq!(output.peak, 0.);
        assert_eq!(
            output.wav_sha256.as_ref().unwrap(),
            &chart_engine::models::hash_file(&out.join(&output.filename)).unwrap()
        );
    }
    let saved: larsnet::SeparationReceipt =
        serde_json::from_slice(&fs::read(out.join(RECEIPT_FILENAME)).unwrap()).unwrap();
    assert_eq!(saved.source_pcm_sha256, receipt.source_pcm_sha256);
    assert!(!saved.warnings.is_empty());
    let cancelled = temp.path().join("cancelled-after-native-block");
    let error = larsnet::separate_files_with_progress(
        &source,
        Path::new(&model_dir),
        &cancelled,
        |done, total| {
            if total >= 5 && done >= 1 {
                anyhow::bail!("cancel after first native block");
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cancel after first native block")
    );
    assert!(!cancelled.exists());
}
