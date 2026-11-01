//! Browser vocal models feed the same source-bound Rust vocal generator.
use crate::{
    models,
    onnx::{Input, Runtime},
};
use anyhow::{Context, Result, ensure};
use std::path::Path;
pub async fn generate(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    timing: &crate::timing::TempoMap,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_generation::Generation> {
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Vocals)
        .context("No vocal stem")?;
    manifest.validate_binding(&models::hash_file(audio)?, manifest.frames as f64 / 44100.)?;
    ensure!(
        models::hash_file(&stem.path)? == stem.sha256,
        "Vocal stem changed"
    );
    let candidates = crate::transcription::pitch(&stem.path, progress).await?;
    let neural = crate::vocal_generation::NeuralPitchEvidence {
        duration_seconds:manifest.frames as f64/44100.,
        model_sha256: models::assets()
            .into_iter()
            .find(|a| a.id == "pitch")
            .unwrap()
            .sha256,
        audio_sha256: manifest.source_audio_sha256.clone(),
        vocal_stem_sha256: stem.sha256.clone(),
        implementation_sha256: crate::vocal_generation::neural_pitch_implementation_hash(),
        notes: candidates.into_iter().map(|e| e.candidate).collect(),
    };
    let pcm = models::decode(&stem.path, 16000, 1)?;
    let mut evidence = swift(&pcm, progress).await?;
    evidence.audio_sha256 = manifest.source_audio_sha256.clone();
    evidence.vocal_stem_sha256 = stem.sha256.clone();
    evidence.duration_seconds = manifest.frames as f64 / 44100.;
    progress("Assembling source-bound vocal pitches and review phrases".into());
    crate::vocal_generation::generate(crate::vocal_generation::Request {
        audio,
        vocal_stem: &stem.path,
        alignment: &crate::clean::StemAlignment {
            audio_sha256: manifest.source_audio_sha256.clone(),
            stem_sha256: stem.sha256.clone(),
            evidence: manifest.alignment.details.clone(),
        },
        timing,
        words: None,
        lyrics: None,
        policy: &Default::default(),
        neural_pitch: Some(&neural),
        vocal_pitch: Some(&evidence),
    })
}

async fn swift(
    pcm: &[f32],
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_pitch::Evidence> {
    use crate::vocal_pitch::{HOP, decode_chunk, finish_pcm};
    ensure!(
        !pcm.is_empty() && pcm.len() <= 16000 * 3600,
        "Invalid vocal pitch source length"
    );
    let model = Runtime::load("vocal-pitch", progress, false).await?;
    let count = (pcm.len() / HOP).max(1);
    let mut frames = Vec::with_capacity(count);
    for start in (0..count).step_by(1875) {
        progress(format!(
            "Detecting vocal contour · chunk {} of {}",
            start / 1875 + 1,
            count.div_ceil(1875)
        ));
        let end = (start + 1875).min(count);
        let left = start.saturating_sub(11);
        let tail = if end < count {
            ((end + 10) * HOP).min(pcm.len())
        } else {
            pcm.len()
        };
        let samples = &pcm[left * HOP..tail];
        let outputs = model
            .run_inputs(&[
                Input {
                    name: "audio",
                    values: samples,
                    shape: &[1, samples.len()],
                },
                Input {
                    name: "fmin",
                    values: &[46.875],
                    shape: &[],
                },
                Input {
                    name: "fmax",
                    values: &[2093.75],
                    shape: &[],
                },
            ])
            .await?;
        let (pitch_shape, pitches) = Runtime::output_f64(&outputs, "pitch")?;
        let confidence = Runtime::output(&outputs, "confidence")?;
        ensure!(
            pitch_shape.len() == 2
                && pitch_shape[0] == 1
                && confidence.shape == pitch_shape
                && pitches.len() >= end - left,
            "Unexpected vocal pitch output contract"
        );
        frames.extend(decode_chunk(
            pcm,
            start,
            end,
            left,
            &pitches,
            &confidence.values,
        )?);
    }
    finish_pcm(pcm, frames)
}
