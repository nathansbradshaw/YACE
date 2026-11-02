//! Optional HQ Piano model; all spectral/overlap math is shared with desktop.
pub use crate::keyboard_separation_asset::*;
pub use crate::keyboard_separation_dsp::*;
use crate::{
    models,
    onnx::Runtime,
    stem_review::{AlignmentEvidence, SeparationIdentity, StemAsset, StemManifest},
};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
pub fn implementation_hash() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            [
                include_bytes!("keyboard_separation.rs").as_slice(),
                include_bytes!("../../src/keyboard_separation_dsp.rs")
            ]
            .concat()
        )
    )
}
pub async fn run(
    audio: &Path,
    output: &Path,
    progress: &mut impl FnMut(String),
) -> Result<StemManifest> {
    let source_hash = models::hash_file(audio)?;
    let pcm = models::decode(audio, SAMPLE_RATE, 2)?;
    let plan = ChunkPlan::new(pcm.len() / 2)?;
    ensure!(
        plan.source_frames
            .checked_mul(48)
            .is_some_and(|bytes| bytes <= 1024 * 1024 * 1024),
        "HQ separation output exceeds the browser's 1 GiB working-set limit"
    );
    let model = Runtime::load(MODEL_ID, progress, false).await?;
    let mut overlap = Overlap::new();
    let mut stems: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::with_capacity(pcm.len()));
    let mut seconds = Vec::new();
    for (number, &start) in plan.starts.iter().enumerate() {
        let timer = js_sys::Date::now();
        progress(format!(
            "Separating higher-quality piano · chunk {} of {}",
            number + 1,
            plan.starts.len()
        ));
        let mut chunk: [Vec<f32>; 2] = std::array::from_fn(|_| vec![0.; CHUNK_SAMPLES]);
        for (ch, channel) in chunk.iter_mut().enumerate() {
            for (i, value) in channel.iter_mut().enumerate() {
                if let Some(j) = plan.source_index(start, i) {
                    *value = pcm[j * 2 + ch];
                }
            }
        }
        let (input, original) = stft(&chunk)?;
        drop(chunk);
        let result = model
            .run("input", &input, &[1, TIME_FRAMES, FREQUENCIES * 4])
            .await?;
        drop(input);
        let masks = Runtime::output(&result, "output")?;
        ensure!(
            masks.shape == [1, 6, FREQUENCIES * 2, TIME_FRAMES, 2],
            "Unexpected HQ output shape"
        );
        let available = (plan.padded - start).min(CHUNK_SAMPLES);
        let weights: Vec<_> = (0..available).map(|i| plan.weight(start, i)).collect();
        for s in 0..6 {
            for ch in 0..2 {
                let data = reconstruct(&original, Some((&masks.values, s)), ch)?;
                overlap.add(s * 2 + ch, &data, &weights, available)?;
            }
        }
        for (i, &weight) in weights.iter().enumerate() {
            overlap.counter[i] += weight;
        }
        let until = plan
            .starts
            .get(number + 1)
            .copied()
            .unwrap_or(plan.border + plan.source_frames);
        overlap.flush(&plan, start, until, |s, l, r| {
            stems[s].extend([l, r]);
            Ok(())
        })?;
        seconds.push((js_sys::Date::now() - timer) / 1000.);
    }
    ensure!(
        models::hash_file(audio)? == source_hash,
        "HQ source changed during inference"
    );
    let mut assets = Vec::new();
    let mut hashes = BTreeMap::new();
    for (id, samples) in STEM_ORDER.into_iter().zip(stems) {
        ensure!(
            samples.len() == pcm.len() && samples.iter().all(|s| s.is_finite()),
            "Incomplete HQ source clock"
        );
        let path = output.join(format!("{}.wav", id.as_str()));
        let wave = crate::audio_decode::Pcm {
            stereo: samples,
            rate: SAMPLE_RATE,
            channels: 2,
        }
        .float_wave()?;
        crate::storage::write_owned(&path, wave)?;
        let hash = models::hash_file(&path)?;
        hashes.insert(id.as_str().into(), hash.clone());
        assets.push(StemAsset {
            id,
            path,
            sha256: hash,
        });
    }
    let mut generation = SeparationIdentity {
        model_id: MODEL_ID.into(),
        model_sha256: MODEL_SHA256.into(),
        backend: "bs-roformer-sw-ort-browser-wasm-fp32-v1".into(),
        parameters: BTreeMap::from([
            ("browser_code_sha256".into(), implementation_hash()),
            (
                "runtime".into(),
                "ONNX Runtime Web 1.22.0 CPU SIMD, one thread".into(),
            ),
            (
                "source_clock".into(),
                "44100Hz stereo; entire source; no trim, shift, normalization or limiter".into(),
            ),
            ("weights_license".into(), WEIGHT_LICENSE_NOTICE.into()),
        ]),
        cache_key: String::new(),
    };
    generation.cache_key = generation.calculated_cache_key(&source_hash)?;
    let receipt = Receipt {
        version: 1,
        source_audio_sha256: source_hash.clone(),
        source_frames: plan.source_frames as u64,
        sample_rate: SAMPLE_RATE,
        channels: 2,
        generation: generation.clone(),
        stem_sha256: hashes,
        chunk_starts: plan.starts.iter().map(|s| *s as u64).collect(),
        chunk_seconds: seconds,
        skipped_padding_chunks: plan.skipped_padding_chunks as u64,
    };
    crate::storage::write(
        output.join(RECEIPT_FILENAME),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    let manifest=StemManifest{version:1,source_audio_sha256:source_hash,sample_rate:SAMPLE_RATE,channels:2,frames:plan.source_frames as u64,alignment:AlignmentEvidence{method:"Native Rust HQ spectral reconstruction on browser WASM".into(),details:"Shared desktop STFT, complex masks, inverse transform, reflected chunk plan and overlap; all six full-source clocks and hashes verified".into()},stems:assets,generation:Some(generation)};
    let mut portable = manifest.clone();
    for stem in &mut portable.stems {
        stem.path = stem.path.strip_prefix(output)?.into();
    }
    crate::storage::write(
        output.join("stem-manifest.json"),
        serde_json::to_vec_pretty(&portable)?,
    )?;
    models::remember_stems(&manifest)?;
    Ok(manifest)
}
