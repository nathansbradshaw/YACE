//! Pinned optional SwiftF0 evaluation through native ONNX Runtime. The model
//! and chunk/silence conventions are MIT, Lars Nieradzik, revision below.
//! Raw frames are evidence, not automatically accepted singing or chart notes.
use crate::{clean::StemAlignment, models};
use anyhow::{Result, ensure};
use ort::{session::Session, value::Tensor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MODEL_ID: &str = "swift-f0-v0.3.0";
pub const REVISION: &str = "d1ba77fe5331310b778ebe126b6cb3656d41d24f";
pub const MODEL_SHA256: &str = "6385e8c2ebc3872e82c9ff5946228de44cd3be77a750ea53698b7dbfe94b0a22";
pub const MODEL_URL: &str = "https://raw.githubusercontent.com/lars76/swift-f0/d1ba77fe5331310b778ebe126b6cb3656d41d24f/swift_f0/model.onnx";
const RATE: usize = 16000;
const HOP: usize = 256;
const CHUNK_FRAMES: usize = 1875;
const LEFT_FRAMES: usize = 11;
const RIGHT_FRAMES: usize = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub time_seconds: f64,
    pub frequency_hz: Option<f64>,
    pub confidence: f32,
    pub rms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub version: u32,
    pub model_id: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub implementation_sha256: String,
    pub audio_sha256: String,
    pub vocal_stem_sha256: String,
    pub pcm_sha256: String,
    pub duration_seconds: f64,
    pub sample_rate: usize,
    pub hop_samples: usize,
    pub frames: Vec<Frame>,
}

pub fn implementation_hash() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("vocal_pitch.rs"));
    hash.update(include_bytes!("models.rs"));
    hash.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

pub fn analyze(
    audio: &Path,
    stem: &Path,
    alignment: &StemAlignment,
    model: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Evidence> {
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    let audio_sha = models::hash_file(audio)?;
    let stem_sha = models::hash_file(stem)?;
    ensure!(
        alignment.audio_sha256.eq_ignore_ascii_case(&audio_sha)
            && alignment.stem_sha256.eq_ignore_ascii_case(&stem_sha)
            && !alignment.evidence.trim().is_empty(),
        "Vocal pitch source binding does not match"
    );
    // Reject obviously over-budget files before allocating both full-length
    // stereo buffers. Decoded sample counts below remain the clock authority.
    for source in [audio, stem] {
        let metadata = crate::pipeline::metadata(source)?;
        ensure!(
            metadata.duration.is_finite() && (0. ..=3600.).contains(&metadata.duration),
            "Vocal pitch analysis requires finite source/stem durations within the one-hour budget"
        );
    }
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    let original = models::decode(audio, 44100, 2)?;
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    let full_stem = models::decode(stem, 44100, 2)?;
    ensure!(
        !original.is_empty() && original.len() == full_stem.len(),
        "Vocal pitch source/stem clocks differ"
    );
    let duration = original.len() as f64 / 88200.;
    drop(original);
    drop(full_stem);
    let pcm = models::decode(stem, RATE as u32, 1)?;
    let mut evidence = infer_pcm(&pcm, model, cancelled)?;
    ensure!(
        (pcm.len() as f64 / RATE as f64 - duration).abs() <= 1. / RATE as f64,
        "Vocal pitch resampling changed the source clock"
    );
    ensure!(
        models::hash_file(audio)? == audio_sha && models::hash_file(stem)? == stem_sha,
        "Vocal pitch inputs changed during analysis"
    );
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    evidence.audio_sha256 = audio_sha;
    evidence.vocal_stem_sha256 = stem_sha;
    evidence.duration_seconds = duration;
    Ok(evidence)
}

/// Low-level source-PCM analysis for reproducible controls. File-based callers
/// use analyze() to bind frames to the original recording and aligned stem.
pub fn infer_pcm(pcm: &[f32], model: &Path, cancelled: &dyn Fn() -> bool) -> Result<Evidence> {
    ensure!(
        !pcm.is_empty() && pcm.iter().all(|sample| sample.is_finite()),
        "Pitch PCM must be finite and nonempty"
    );
    ensure!(
        pcm.len() <= RATE * 3600,
        "Vocal pitch analysis exceeds one-hour input budget"
    );
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    ensure!(
        models::hash_file(model)? == MODEL_SHA256,
        "Optional vocal pitch model checksum mismatch"
    );
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model)?;
    let count = (pcm.len() / HOP).max(1);
    let mut frames = Vec::with_capacity(count);
    for start in (0..count).step_by(CHUNK_FRAMES) {
        ensure!(!cancelled(), "Vocal pitch analysis cancelled");
        let end = (start + CHUNK_FRAMES).min(count);
        let left = start.saturating_sub(LEFT_FRAMES);
        let tail = if end < count {
            ((end + RIGHT_FRAMES) * HOP).min(pcm.len())
        } else {
            pcm.len()
        };
        let values = pcm[left * HOP..tail].to_vec();
        let input = Tensor::from_array(([1, values.len()], values.into_boxed_slice()))?;
        let fmin = Tensor::from_array((Vec::<usize>::new(), vec![46.875f32].into_boxed_slice()))?;
        let fmax = Tensor::from_array((Vec::<usize>::new(), vec![2093.75f32].into_boxed_slice()))?;
        let output = session.run(ort::inputs!["audio" => input, "fmin" => fmin, "fmax" => fmax])?;
        ensure!(!cancelled(), "Vocal pitch analysis cancelled");
        let (pitch_shape, pitches) = output["pitch"].try_extract_tensor::<f64>()?;
        let (confidence_shape, confidences) = output["confidence"].try_extract_tensor::<f32>()?;
        ensure!(
            pitch_shape.len() == 2
                && pitch_shape[0] == 1
                && confidence_shape == pitch_shape
                && pitches.len() == confidences.len()
                && pitches.len() >= end - left,
            "Unexpected optional vocal pitch tensor contract"
        );
        for global in start..end {
            let local = global - left;
            let pitch = pitches[local];
            let mut confidence = confidences[local];
            ensure!(
                confidence.is_finite() && (0. ..=1.).contains(&confidence),
                "Nonfinite or uncalibrated vocal pitch confidence"
            );
            let lo = (global * HOP).min(pcm.len());
            let hi = ((global + 1) * HOP).min(pcm.len());
            let peak = pcm[lo..hi]
                .iter()
                .map(|sample| sample.abs())
                .fold(0f32, f32::max);
            if peak < 1e-3 {
                confidence = 0.;
            }
            let energy_start = global.saturating_sub(1) * HOP;
            let power: f64 = pcm[energy_start..hi]
                .iter()
                .map(|sample| (*sample as f64).powi(2))
                .sum();
            let frequency_hz =
                (pitch.is_finite() && (46.875..=2093.75).contains(&pitch) && confidence > 0.)
                    .then_some(pitch);
            frames.push(Frame {
                time_seconds: global as f64 * HOP as f64 / RATE as f64,
                frequency_hz,
                confidence,
                rms: (power / (2 * HOP) as f64).sqrt(),
            });
        }
    }
    ensure!(
        frames.len() == count && models::hash_file(model)? == MODEL_SHA256,
        "Vocal pitch model or frame count changed during analysis"
    );
    let mut pcm_hash = Sha256::new();
    for sample in pcm {
        pcm_hash.update(sample.to_le_bytes());
    }
    ensure!(!cancelled(), "Vocal pitch analysis cancelled");
    Ok(Evidence {
        version: 1,
        model_id: MODEL_ID.into(),
        model_revision: REVISION.into(),
        model_sha256: MODEL_SHA256.into(),
        implementation_sha256: implementation_hash(),
        audio_sha256: String::new(),
        vocal_stem_sha256: String::new(),
        pcm_sha256: format!("{:x}", pcm_hash.finalize()),
        duration_seconds: pcm.len() as f64 / RATE as f64,
        sample_rate: RATE,
        hop_samples: HOP,
        frames,
    })
}
