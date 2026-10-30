use chart_engine::{keyboard_separation as ks, models};
use ort::{
    tensor::{Shape, SymbolicDimensions, TensorElementType},
    value::ValueType,
};
fn tensor(shape: &[i64], ty: TensorElementType) -> ValueType {
    ValueType::Tensor {
        ty,
        shape: Shape::new(shape.iter().copied()),
        dimension_symbols: SymbolicDimensions::new(shape.iter().map(|_| String::new())),
    }
}
#[test]
fn optional_asset_is_pinned_and_absent_from_default_catalog() {
    let asset = ks::asset();
    assert_eq!(asset.id, ks::MODEL_ID);
    assert_eq!(asset.filename, "bs_roformer_sw.onnx");
    assert_eq!(ks::MODEL_BYTES, 704536949);
    assert_eq!(
        asset.sha256,
        "d2a13ad34eb26a8aa3135314b2cf1efb41e70289ffb9d03124a4201736a56df3"
    );
    assert!(asset.url.contains(ks::MODEL_REVISION));
    assert!(asset.license.contains("unresolved"));
    assert!(!models::assets().iter().any(|a| a.id == ks::MODEL_ID));
}
#[test]
fn corrupt_existing_optional_asset_is_never_replaced_or_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(ks::MODEL_FILENAME);
    std::fs::write(&path, b"preserve invalid local bytes").unwrap();
    assert!(ks::verified(dir.path()).is_err());
    assert!(ks::fetch(dir.path()).is_err());
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"preserve invalid local bytes"
    );
}
#[test]
fn explicit_and_packaged_existing_model_paths_remain_exact_and_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let custom = root.path().join("custom models");
    assert_eq!(
        ks::resolved_model_path(&custom).unwrap(),
        custom.join(ks::MODEL_FILENAME)
    );
    assert!(!custom.exists());
    let bundle = root.path().join("YACE.app/Contents/Resources/models");
    std::fs::create_dir_all(&bundle).unwrap();
    let path = bundle.join(ks::MODEL_FILENAME);
    std::fs::write(&path, b"preserve invalid packaged bytes").unwrap();
    assert_eq!(ks::resolved_model_path(&bundle).unwrap(), path);
    assert!(ks::verified(&bundle).is_err());
    assert!(ks::fetch(&bundle).is_err());
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"preserve invalid packaged bytes"
    );
}
#[test]
fn contract_rejects_wrong_dtype_dynamic_axes_and_waveform_export() {
    let input = tensor(&[1, 1151, 4100], TensorElementType::Float32);
    let output = tensor(&[1, 6, 2050, 1151, 2], TensorElementType::Float32);
    ks::validate_contract(&input, &output).unwrap();
    for bad in [
        tensor(&[1, 1150, 4100], TensorElementType::Float32),
        tensor(&[-1, 1151, 4100], TensorElementType::Float32),
        tensor(&[1, 2, 131072], TensorElementType::Float32),
        tensor(&[1, 1151, 4100], TensorElementType::Float64),
    ] {
        assert!(ks::validate_contract(&bad, &output).is_err());
    }
    for bad in [
        tensor(&[1, 6, 2, 131072], TensorElementType::Float32),
        tensor(&[1, 6, 2050, 1151, 2], TensorElementType::Int64),
        tensor(&[1, 5, 2050, 1151, 2], TensorElementType::Float32),
    ] {
        assert!(ks::validate_contract(&input, &bad).is_err());
    }
}
#[test]
fn source_clock_coverage_and_padding_skip_are_exact() {
    for frames in [
        2,
        12000,
        ks::STEP_SAMPLES - 1,
        ks::STEP_SAMPLES,
        ks::STEP_SAMPLES + 1,
        ks::CHUNK_SAMPLES,
        ks::CHUNK_SAMPLES + 1,
        1323000,
        ks::CHUNK_SAMPLES * 3 + 19,
    ] {
        let plan = ks::ChunkPlan::new(frames).unwrap();
        assert!(plan.starts.iter().all(|s| *s < plan.border + frames));
        for source in [0, 1, frames / 2, frames - 2, frames - 1] {
            let point = source + plan.border;
            let old: f64 = (0..plan.padded)
                .step_by(ks::STEP_SAMPLES)
                .filter(|s| point >= *s && point - s < ks::CHUNK_SAMPLES)
                .map(|s| plan.weight(s, point - s))
                .sum();
            let kept: f64 = plan
                .starts
                .iter()
                .filter(|s| point >= **s && point - **s < ks::CHUNK_SAMPLES)
                .map(|s| plan.weight(*s, point - s))
                .sum();
            assert_eq!(kept, old);
            assert!(kept > 0.);
        }
    }
    let p = ks::ChunkPlan::new(1323000).unwrap();
    assert_eq!(p.starts.len(), 6);
    assert_eq!(p.skipped_padding_chunks, 1);
    assert!(ks::ChunkPlan::new(0).is_err());
    assert!(ks::ChunkPlan::new(1).is_err());
    assert!(ks::ChunkPlan::new(usize::MAX).is_err());
}
#[test]
fn stft_layout_and_complex_reconstruction_preserve_endpoints() {
    let pcm: [Vec<f32>; 2] = std::array::from_fn(|ch| {
        (0..ks::CHUNK_SAMPLES)
            .map(|i| {
                if i == 0 || i == ks::CHUNK_SAMPLES - 1 {
                    0.7
                } else {
                    (std::f64::consts::TAU * (220. + 110. * ch as f64) * i as f64 / 44100.).sin()
                        as f32
                        * 0.2
                }
            })
            .collect()
    });
    let (input, spectrum) = ks::stft(&pcm).unwrap();
    for ch in 0..2 {
        let reconstructed = ks::reconstruct(&spectrum, None, ch).unwrap();
        let max = pcm[ch]
            .iter()
            .zip(reconstructed)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max < 2e-6, "{max}");
        for t in [0, ks::TIME_FRAMES - 1] {
            for f in [0, ks::FREQUENCIES - 1] {
                let z = spectrum[(ch * ks::FREQUENCIES + f) * ks::TIME_FRAMES + t];
                let at = t * ks::FREQUENCIES * 4 + (f * 2 + ch) * 2;
                assert_eq!(input[at], z.re);
                assert_eq!(input[at + 1], z.im);
            }
        }
    }
    assert!(ks::reconstruct(&spectrum, Some((&[0.; 1], 0)), 0).is_err());
    assert!(ks::reconstruct(&spectrum, None, 2).is_err());
    let invalid = [
        vec![f32::NAN; ks::CHUNK_SAMPLES],
        vec![0.; ks::CHUNK_SAMPLES],
    ];
    assert!(ks::stft(&invalid).is_err());
}
#[test]
fn output_transaction_does_not_touch_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let keep = dir.path().join("keep.wav");
    std::fs::write(&keep, b"recovery").unwrap();
    assert!(
        ks::separate(
            std::path::Path::new("missing-source"),
            std::path::Path::new("missing-model"),
            dir.path()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(keep).unwrap(), b"recovery");
}
#[test]
#[ignore = "requires pinned local 704MB model and frozen Bedroom prototype receipt; genuine native inference"]
fn native_streamed_port_matches_frozen_prototype_pcm() {
    use chart_engine::stem_review::{StemId, StemManifest};
    let research = std::env::var_os("YACE_HQ_RESEARCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("output/official-keys-audit/roformer-research")
        });
    let source = research
        .parent()
        .unwrap()
        .join("songs/bedroom/full-mix-0-30.wav");
    let dir = tempfile::tempdir().unwrap();
    ks::separate(&source, &research.join(ks::MODEL_FILENAME), dir.path()).unwrap();
    let receipt: ks::Receipt =
        serde_json::from_slice(&std::fs::read(dir.path().join(ks::RECEIPT_FILENAME)).unwrap())
            .unwrap();
    assert_eq!(receipt.source_frames, 1323000);
    assert_eq!(receipt.chunk_starts.len(), 6);
    assert_eq!(receipt.skipped_padding_chunks, 1);
    assert_eq!(receipt.generation.model_sha256, ks::MODEL_SHA256);
    receipt
        .generation
        .validate(&receipt.source_audio_sha256)
        .unwrap();
    for id in StemId::ALL {
        let new =
            models::decode(&dir.path().join(format!("{}.wav", id.as_str())), 44100, 2).unwrap();
        let old = models::decode(
            &research.join(format!("bedroom/{}.wav", id.as_str())),
            44100,
            2,
        )
        .unwrap();
        assert_eq!(new.len(), old.len());
        let max = new
            .iter()
            .zip(&old)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max <= 2e-6, "{} {max}", id.as_str());
        eprintln!("{} max PCM delta {max}", id.as_str());
    }
    let manifest=StemManifest::build(&source,StemId::ALL.into_iter().map(|id|(id,dir.path().join(format!("{}.wav",id.as_str())))).collect(),chart_engine::stem_review::AlignmentEvidence{method:"production port parity validation".into(),details:"Native real ONNX inference; source frame and PCM parity checked against the frozen prototype; no acoustic accuracy assertion.".into()}).unwrap();
    assert_eq!(manifest.frames, 1323000);
    eprintln!(
        "Native production separator parity passed: six stems, {} frames, six contributing chunks",
        manifest.frames
    );
}
