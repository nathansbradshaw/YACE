//! Optional local LarsNet checkpoints. Share the native FFT, masks and clock;
//! load one ONNX session at a time to bound the browser's inference resources.
#[path = "../../src/larsnet.rs"]
mod frontend;
use anyhow::{Context, Result, ensure};
pub use frontend::*;
use frontend::{
    BINS, BLOCK, FFT, HOP, denominator, pcm_hash, sanitized_mask, spectrum_block, synthesize,
    validate_input, window,
};
use rustfft::FftPlanner;
use std::path::Path;

pub async fn separate_to_directory(
    input: &[f32],
    model_dir: &Path,
    directory: &Path,
    progress: &mut impl FnMut(String),
) -> Result<SeparationReceipt> {
    let started = js_sys::Date::now();
    let frames = validate_input(input)?;
    // Verify the complete set before creating any session or result.
    for (part, expected) in MODELS {
        let path = model_dir.join(format!("{part}.onnx"));
        ensure!(
            crate::storage::shared(&path)?.len() as u64 == MODEL_BYTES,
            "Invalid LarsNet model length: {part}"
        );
        ensure!(
            crate::models::hash_file(&path)? == expected,
            "LarsNet model checksum differs: {part}"
        );
    }
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT);
    let ifft = planner.plan_fft_inverse(FFT);
    let win = window();
    let divisor = denominator(frames, &win);
    let blocks = (frames.div_ceil(HOP) + 1).div_ceil(BLOCK);
    let mut outputs = Vec::new();
    for (part, _) in MODELS {
        progress(format!("Loading local LarsNet {part} model…"));
        let bytes = crate::storage::shared(model_dir.join(format!("{part}.onnx")))?;
        let runtime = crate::onnx::Runtime::from_bytes(&bytes, false).await?;
        let mut samples = vec![0.; input.len()];
        let mut numeric = MaskNumeric {
            raw_min: f32::INFINITY,
            raw_max: f32::NEG_INFINITY,
            clipped_values: 0,
        };
        let inference = async {
            for block in 0..blocks {
                progress(format!("Splitting {part}: block {} of {blocks}", block + 1));
                let (original, magnitude, count) =
                    spectrum_block(input, block * BLOCK, &win, fft.as_ref());
                let result = runtime
                    .run("magnitude", &magnitude, &[1, 2, BINS, BLOCK])
                    .await?;
                let output = crate::onnx::Runtime::output(&result, "mask")?;
                ensure!(
                    output.shape == [1, 2, BINS, BLOCK],
                    "Unexpected LarsNet {part} output shape"
                );
                let mask = sanitized_mask(&output.values, &mut numeric)
                    .with_context(|| format!("LarsNet {part} block {block}"))?;
                synthesize(
                    &original,
                    &mask,
                    block * BLOCK,
                    count,
                    &win,
                    ifft.as_ref(),
                    &mut samples,
                );
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        // Release even when a graph fails its shape or finite-mask checks.
        let released = runtime.release().await;
        inference?;
        released?;
        for (index, frame) in samples.chunks_exact_mut(2).enumerate() {
            ensure!(divisor[index] > 0., "Uncovered synthesis frame");
            for sample in frame {
                *sample /= divisor[index];
            }
        }
        ensure!(
            samples.iter().all(|v| v.is_finite()),
            "Nonfinite separated audio"
        );
        let mut receipt = OutputReceipt {
            part: part.into(),
            filename: format!("{part}.wav"),
            frames: frames as u64,
            pcm_sha256: pcm_hash(&samples),
            wav_sha256: None,
            rms: (samples.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / samples.len() as f64)
                .sqrt(),
            peak: samples.iter().map(|v| (*v as f64).abs()).fold(0., f64::max),
            mask_numeric: numeric,
        };
        let path = directory.join(&receipt.filename);
        ensure!(!crate::storage::exists(&path), "Kit output already exists");
        let wav = crate::audio_decode::Pcm {
            stereo: samples,
            rate: 44100,
            channels: 2,
        }
        .float_wave()?;
        crate::storage::write_owned(&path, wav)?;
        receipt.wav_sha256 = Some(crate::models::hash_file(&path)?);
        outputs.push(receipt);
    }
    Ok(SeparationReceipt {
        version:1,algorithm:ALGORITHM.into(),implementation_sha256:implementation_hash(),
        source_audio_sha256:None,source_pcm_sha256:pcm_hash(input),sample_rate:SAMPLE_RATE,channels:2,
        frames:frames as u64,models:MODELS.into_iter().map(|(id,hash)|(id.into(),hash.into())).collect(),
        weight_license:WEIGHT_LICENSE.into(),outputs,elapsed_seconds:(js_sys::Date::now()-started)/1000.,
        warnings:vec![
            "Estimated drum components can contain bleed, omissions and artifacts; not approved physical-instrument truth.".into(),
            "Independent Rust graph/STFT implementation using browser ONNX Runtime; upstream LibTorch numerical parity and professional-chart quality improvement have not been established.".into(),
            "Five non-normalized masks; component sum need not reconstruct the original drum mixture.".into(),
        ],
    })
}
