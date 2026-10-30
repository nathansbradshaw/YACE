use chart_engine::vocal_pitch;
use std::{f64::consts::TAU, path::PathBuf};

#[test]
fn malformed_or_cancelled_pitch_inputs_fail_before_loading_models() {
    let missing = PathBuf::from("missing-pinned-vocal-model.onnx");
    assert!(vocal_pitch::infer_pcm(&[], &missing, &|| false).is_err());
    assert!(vocal_pitch::infer_pcm(&[f32::NAN], &missing, &|| false).is_err());
    assert!(vocal_pitch::infer_pcm(&[0.; 256], &missing, &|| true).is_err());
}

#[test]
#[ignore = "requires the pinned optional SwiftF0 model via YACE_SWIFT_F0_MODEL"]
fn pinned_model_preserves_source_clock_silence_pitch_and_chunk_seams() {
    let model = PathBuf::from(std::env::var_os("YACE_SWIFT_F0_MODEL").expect("pinned model path"));
    let rate = 16000;
    let mut pcm = vec![0f32; 32 * rate];
    for (index, sample) in pcm.iter_mut().enumerate() {
        let time = index as f64 / rate as f64;
        if (0.2..0.7).contains(&time) || (29.5..31.5).contains(&time) {
            *sample = (0.2 * (TAU * 220. * time).sin() + 0.05 * (TAU * 440. * time).sin()) as f32;
        }
    }
    let result = vocal_pitch::infer_pcm(&pcm, &model, &|| false).unwrap();
    assert_eq!(result.frames.len(), 2000);
    assert_eq!(result.duration_seconds, 32.);
    assert_eq!(result.model_sha256, vocal_pitch::MODEL_SHA256);
    assert!(
        result.audio_sha256.is_empty(),
        "raw PCM controls cannot invent file provenance"
    );
    for (index, frame) in result.frames.iter().enumerate() {
        assert!((frame.time_seconds - index as f64 * 0.016).abs() < 1e-12);
        if (1. ..29.).contains(&frame.time_seconds) {
            assert_eq!(frame.confidence, 0.);
            assert!(frame.frequency_hz.is_none());
        }
    }
    for time in [0.4, 29.8, 30., 30.2, 31.2] {
        let frame = &result.frames[(time / 0.016) as usize];
        assert!(
            frame.confidence >= 0.8,
            "stable tone confidence at {time}: {frame:?}"
        );
        assert!(
            (12. * (frame.frequency_hz.unwrap() / 220.).log2()).abs() < 0.3,
            "stable tone pitch at {time}: {frame:?}"
        );
    }
    let short = vocal_pitch::infer_pcm(&[0.; 1], &model, &|| false).unwrap();
    assert_eq!(short.frames.len(), 1);
    assert_eq!(short.frames[0].confidence, 0.);
}

#[test]
#[ignore = "requires the pinned optional SwiftF0 model via YACE_SWIFT_F0_MODEL"]
fn chunked_pitch_matches_one_whole_graph_across_nonstationary_seam() {
    use ort::{session::Session, value::Tensor};
    let model = PathBuf::from(std::env::var_os("YACE_SWIFT_F0_MODEL").expect("pinned model path"));
    // An unchanging tone cannot expose an incorrect crop by one model frame.
    // These pitch, amplitude and silence transitions surround the 30 s seam.
    let pcm: Vec<f32> = (0..32 * 16000 + 37)
        .map(|index| {
            let t = index as f64 / 16000.;
            if (29.992..30.).contains(&t) {
                return 0.;
            }
            let hz = if t < 29.984 {
                220.
            } else if t < 30.016 {
                330.
            } else if t < 30.192 {
                196.
            } else {
                261.625565
            };
            let amplitude = if t < 29.968 { 0.04 } else { 0.2 };
            (amplitude * ((TAU * hz * t).sin() + 0.2 * (TAU * hz * 2. * t).sin())) as f32
        })
        .collect();
    let chunked = vocal_pitch::infer_pcm(&pcm, &model, &|| false).unwrap();
    let mut session = Session::builder()
        .unwrap()
        .with_intra_threads(2)
        .unwrap()
        .commit_from_file(&model)
        .unwrap();
    let input = Tensor::from_array(([1, pcm.len()], pcm.clone().into_boxed_slice())).unwrap();
    let fmin =
        Tensor::from_array((Vec::<usize>::new(), vec![46.875f32].into_boxed_slice())).unwrap();
    let fmax =
        Tensor::from_array((Vec::<usize>::new(), vec![2093.75f32].into_boxed_slice())).unwrap();
    let output = session
        .run(ort::inputs!["audio"=>input,"fmin"=>fmin,"fmax"=>fmax])
        .unwrap();
    let (_, pitch) = output["pitch"].try_extract_tensor::<f64>().unwrap();
    let (_, confidence) = output["confidence"].try_extract_tensor::<f32>().unwrap();
    assert_eq!(chunked.frames.len(), pcm.len() / 256);
    assert_eq!(pitch.len(), chunked.frames.len());
    assert_eq!(confidence.len(), chunked.frames.len());
    for (index, frame) in chunked.frames.iter().enumerate() {
        assert_eq!(frame.time_seconds, index as f64 * 256. / 16000.);
        let silence = pcm[index * 256..(index + 1) * 256]
            .iter()
            .all(|sample| sample.abs() < 1e-3);
        let expected_confidence = if silence { 0. } else { confidence[index] };
        assert!(
            (frame.confidence - expected_confidence).abs() <= 1e-5,
            "confidence seam/frame {index}: {frame:?} vs {expected_confidence}"
        );
        let expected_pitch = (pitch[index].is_finite()
            && (46.875..=2093.75).contains(&pitch[index])
            && expected_confidence > 0.)
            .then_some(pitch[index]);
        assert_eq!(
            frame.frequency_hz.is_some(),
            expected_pitch.is_some(),
            "voicing seam/frame {index}"
        );
        if let Some(frequency) = frame.frequency_hz {
            assert!(
                (frequency - pitch[index]).abs() <= 1e-4,
                "pitch seam/frame {index}: {frequency} vs {}",
                pitch[index]
            );
        }
        let lo = index.saturating_sub(1) * 256;
        let hi = (index + 1) * 256;
        let expected_rms = (pcm[lo..hi]
            .iter()
            .map(|sample| (*sample as f64).powi(2))
            .sum::<f64>()
            / 512.)
            .sqrt();
        assert!((frame.rms - expected_rms).abs() < 1e-12);
    }
}

#[test]
#[ignore = "requires the pinned optional SwiftF0 model via YACE_SWIFT_F0_MODEL"]
fn cancellation_during_the_final_native_chunk_cannot_return_evidence() {
    use std::cell::Cell;
    let model = PathBuf::from(std::env::var_os("YACE_SWIFT_F0_MODEL").expect("pinned model path"));
    let calls = Cell::new(0usize);
    let result = vocal_pitch::infer_pcm(&vec![0.; 16000], &model, &|| {
        let count = calls.get() + 1;
        calls.set(count);
        count >= 3
    });
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(
        calls.get(),
        3,
        "Cancellation must be observed after the last native run"
    );
}

#[test]
#[ignore = "requires the pinned optional SwiftF0 model via YACE_SWIFT_F0_MODEL"]
fn native_file_evidence_binds_resampled_pcm_and_rejects_source_replacement() {
    use chart_engine::{clean::StemAlignment, models};
    use std::{cell::Cell, fs};
    let model = PathBuf::from(std::env::var_os("YACE_SWIFT_F0_MODEL").expect("pinned model path"));
    let directory = tempfile::tempdir().unwrap();
    let audio = directory.path().join("source.wav");
    let stem = directory.path().join("vocals.wav");
    let vocal: Vec<f32> = (0..22050)
        .map(|i| (0.2 * (TAU * 220. * i as f64 / 44100.).sin()) as f32)
        .collect();
    let mix: Vec<f32> = vocal
        .iter()
        .enumerate()
        .map(|(i, sample)| *sample + (0.05 * (TAU * 110. * i as f64 / 44100.).sin()) as f32)
        .collect();
    models::write_audio(&audio, &mix, &mix).unwrap();
    models::write_audio(&stem, &vocal, &vocal).unwrap();
    let alignment = StemAlignment {
        audio_sha256: models::hash_file(&audio).unwrap(),
        stem_sha256: models::hash_file(&stem).unwrap(),
        evidence: "Synthetic synchronized mix and vocal stem on the same sample clock".into(),
    };
    let bound = vocal_pitch::analyze(&audio, &stem, &alignment, &model, &|| false).unwrap();
    let pcm = models::decode(&stem, 16000, 1).unwrap();
    let raw = vocal_pitch::infer_pcm(&pcm, &model, &|| false).unwrap();
    assert_eq!(bound.audio_sha256, alignment.audio_sha256);
    assert_eq!(bound.vocal_stem_sha256, alignment.stem_sha256);
    assert_eq!(bound.duration_seconds, 0.5);
    assert_eq!(bound.pcm_sha256, raw.pcm_sha256);
    assert_eq!(
        serde_json::to_value(&bound.frames).unwrap(),
        serde_json::to_value(&raw.frames).unwrap()
    );
    let calls = Cell::new(0usize);
    let changed = Cell::new(false);
    let result = vocal_pitch::analyze(&audio, &stem, &alignment, &model, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 5 {
            fs::write(&stem, b"source replaced after PCM capture").unwrap();
            changed.set(true);
        }
        false
    });
    assert!(changed.get());
    assert!(result.unwrap_err().to_string().contains("inputs changed"));
}
