//! Explicit model-dependent smoke tests. Run after `models fetch`:
//! cargo test --test native_models -- --ignored --nocapture
use chart_engine::models;
use std::path::Path;

#[test]
#[ignore = "requires downloaded pretrained ONNX models and FFmpeg"]
fn real_onnx_models_detect_a440_and_reject_silent_drums() {
    let dir = tempfile::tempdir().unwrap();
    let tone = dir.path().join("tone.wav");
    let silence = dir.path().join("silence.wav");
    let wave: Vec<f32> = (0..3 * 44100)
        .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44100.0).sin() * 0.125)
        .collect();
    models::write_audio(&tone, &wave, &wave).unwrap();
    let zeros = vec![0.0; 3 * 44100];
    models::write_audio(&silence, &zeros, &zeros).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let pitches = models::pitch(&tone, &models::verified(&root, "pitch").unwrap()).unwrap();
    assert!(
        pitches
            .iter()
            .any(|p| p.pitch == 69 && p.end - p.start > 1.0),
        "A440 should produce a sustained MIDI 69 note: {pitches:?}"
    );
    assert!(
        models::drums(&silence, &models::verified(&root, "drums").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[cfg(feature = "separation")]
#[test]
#[ignore = "requires downloaded six-stem weights, FFmpeg and a GPU adapter"]
fn native_rust_separator_produces_six_finite_stems_without_changing_input() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("tone.wav");
    let wave: Vec<f32> = (0..44100)
        .map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / 44100.0).sin() * 0.125)
        .collect();
    models::write_audio(&audio, &wave, &wave).unwrap();
    let before = models::hash_file(&audio).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let output = dir.path().join("stems");
    models::separate(
        &audio,
        &models::verified(&root, "separation").unwrap(),
        &output,
    )
    .unwrap();
    for name in ["drums", "bass", "other", "vocals", "guitar", "piano"] {
        let stem = models::decode(&output.join(format!("{name}.wav")), 44100, 2).unwrap();
        assert_eq!(stem.len(), wave.len() * 2);
        assert!(stem.iter().all(|v| v.is_finite()));
    }
    assert_eq!(models::hash_file(&audio).unwrap(), before);
}
