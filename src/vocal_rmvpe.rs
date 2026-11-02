//! Optional pinned RMVPE contour for High accuracy. Native/browser share its
//! mel input, decoder, clock and note-tracking rules. Weights are not bundled.
use crate::vocal_pitch::{Evidence, Frame};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
pub const MODEL_ID: &str = "rmvpe-rvc-v1";
pub const REVISION: &str = "e6d0c1a17da07c33557852f9dfa2bd44cc75737d";
pub const SHA256: &str = "5370e71ac80af8b4b7c793d27efd51fd8bf962de3a7ede0766dac0befa3660fd";
pub const RATE: usize = 16000;
pub const HOP: usize = 160;
pub fn asset() -> crate::models::ModelAsset {
    crate::models::ModelAsset{id:"vocal-rmvpe".into(),filename:"rmvpe.onnx".into(),url:format!("https://huggingface.co/lj1995/VoiceConversionWebUI/resolve/{REVISION}/rmvpe.onnx"),sha256:SHA256.into(),license:"RVC publisher declares MIT; upstream RMVPE source Apache-2.0; separate weight provenance tracked in THIRD_PARTY.md; optional local analysis, no bundled weights".into()}
}
pub fn implementation_hash() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("vocal_rmvpe.rs"));
    h.update(include_bytes!("rmvpe_frontend.rs"));
    h.update(include_bytes!("vocal_pitch_tracking.rs"));
    format!("{:x}", h.finalize())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare(
    directory: &std::path::Path,
    download: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<std::path::PathBuf> {
    let a = asset();
    let directory = crate::models::optional_model_directory(
        directory,
        std::path::Path::new(""),
        &[&a.filename],
    )?;
    let path = directory.join(&a.filename);
    if !path.exists() {
        ensure!(
            download,
            "High accuracy requires the pinned RMVPE model (about 362 MB). Install rmvpe.onnx or enable downloads; no lower-quality mode was substituted."
        );
        crate::models::fetch_selected_assets(&directory, &[a], cancelled)?;
    }
    ensure!(
        crate::models::hash_file(&path)? == SHA256,
        "High accuracy RMVPE model checksum differs"
    );
    Ok(path)
}
#[allow(clippy::excessive_precision)] // Preserve the pinned upstream cents constant.
pub fn decode_chunk(pcm: &[f32], start: usize, values: &[f32]) -> Result<Vec<Frame>> {
    ensure!(
        values.len().is_multiple_of(360),
        "Invalid RMVPE salience length"
    );
    let mut frames = Vec::with_capacity(values.len() / 360);
    for (local, row) in values.chunks_exact(360).enumerate() {
        ensure!(
            row.iter()
                .all(|v| v.is_finite() && (-1e-6..=1. + 1e-6).contains(v)),
            "Invalid RMVPE salience"
        );
        let peak = (0..360).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
        let confidence = row[peak].clamp(0., 1.);
        let (mut total, mut weighted) = (0f64, 0f64);
        for (i, &p) in row
            .iter()
            .enumerate()
            .take((peak + 5).min(360))
            .skip(peak.saturating_sub(4))
        {
            let p = p.clamp(0., 1.) as f64;
            total += p;
            weighted += (20. * i as f64 + 1_997.379_408_437_619_1) * p;
        }
        let frequency_hz =
            (confidence > 0.03 && total > 0.).then(|| 10. * 2f64.powf(weighted / total / 1200.));
        let index = start + local;
        let a = (index * HOP).saturating_sub(HOP / 2).min(pcm.len());
        let b = (index * HOP + HOP / 2).min(pcm.len());
        let rms = if b > a {
            (pcm[a..b].iter().map(|p| (*p as f64).powi(2)).sum::<f64>() / (b - a) as f64).sqrt()
        } else {
            0.
        };
        frames.push(Frame {
            time_seconds: index as f64 * 0.01,
            frequency_hz,
            confidence,
            rms,
        });
    }
    Ok(frames)
}
pub fn finish_pcm(pcm: &[f32], frames: Vec<Frame>) -> Result<Evidence> {
    ensure!(
        pcm.len() > 512
            && pcm.len() <= RATE * 3600
            && pcm.iter().all(|v| v.is_finite())
            && frames.len() == pcm.len() / HOP + 1,
        "RMVPE requires finite audio between 33ms and one hour"
    );
    let mut hash = Sha256::new();
    for p in pcm {
        hash.update(p.to_le_bytes());
    }
    Ok(Evidence {
        version: 1,
        model_id: MODEL_ID.into(),
        model_revision: REVISION.into(),
        model_sha256: SHA256.into(),
        implementation_sha256: implementation_hash(),
        audio_sha256: String::new(),
        vocal_stem_sha256: String::new(),
        pcm_sha256: format!("{:x}", hash.finalize()),
        duration_seconds: pcm.len() as f64 / RATE as f64,
        sample_rate: RATE,
        hop_samples: HOP,
        frames,
    })
}
#[cfg(not(target_arch = "wasm32"))]
pub fn analyze(
    audio: &std::path::Path,
    stem: &std::path::Path,
    binding: &crate::clean::StemAlignment,
    model: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Evidence> {
    use crate::models;
    use ort::session::Session;
    ensure!(!cancelled(), "Vocal analysis cancelled");
    for path in [audio, stem] {
        let m = crate::pipeline::metadata(path)?;
        ensure!(
            m.duration.is_finite() && (0.033..=3600.).contains(&m.duration),
            "RMVPE source exceeds supported duration"
        );
    }
    let source = models::hash_file(audio)?;
    let vocal = models::hash_file(stem)?;
    ensure!(
        source == binding.audio_sha256
            && vocal == binding.stem_sha256
            && !binding.evidence.trim().is_empty(),
        "RMVPE source binding differs"
    );
    ensure!(
        models::hash_file(model)? == SHA256,
        "RMVPE model checksum differs"
    );
    let a = models::decode(audio, 44100, 2)?;
    let b = models::decode(stem, 44100, 2)?;
    ensure!(
        !a.is_empty() && a.len() == b.len(),
        "RMVPE source clocks differ"
    );
    let duration = a.len() as f64 / 88200.;
    drop(a);
    drop(b);
    let pcm = models::decode(stem, 16000, 1)?;
    ensure!(
        pcm.len() > 512 && pcm.len() <= RATE * 3600,
        "RMVPE source exceeds supported length"
    );
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model)?;
    let count = pcm.len() / HOP + 1;
    let mut frames = Vec::with_capacity(count);
    for start in (0..count).step_by(3000) {
        ensure!(!cancelled(), "Vocal analysis cancelled");
        crate::generation_progress::units("Comparing vocal pitch with RMVPE", start, count);
        let end = (start + 3000).min(count);
        let salience = crate::rmvpe_frontend::infer(&mut session, &pcm, start, end)?;
        frames.extend(decode_chunk(&pcm, start, &salience)?);
    }
    let mut evidence = finish_pcm(&pcm, frames)?;
    evidence.audio_sha256 = source.clone();
    evidence.vocal_stem_sha256 = vocal.clone();
    evidence.duration_seconds = duration;
    ensure!(
        !cancelled()
            && models::hash_file(audio)? == source
            && models::hash_file(stem)? == vocal
            && models::hash_file(model)? == SHA256,
        "RMVPE inputs changed or analysis cancelled"
    );
    Ok(evidence)
}
pub fn validate(e: &Evidence, audio: &str, stem: &str, duration: f64) -> Result<()> {
    ensure!(
        e.version == 1
            && e.model_id == MODEL_ID
            && e.model_revision == REVISION
            && e.model_sha256 == SHA256
            && e.implementation_sha256 == implementation_hash()
            && e.audio_sha256.eq_ignore_ascii_case(audio)
            && e.vocal_stem_sha256.eq_ignore_ascii_case(stem)
            && (e.duration_seconds - duration).abs() <= 1. / 44100.
            && e.sample_rate == RATE
            && e.hop_samples == HOP
            && e.pcm_sha256.len() == 64
            && e.pcm_sha256.bytes().all(|v| v.is_ascii_hexdigit()),
        "RMVPE source/model/clock identity differs"
    );
    ensure!(
        !e.frames.is_empty()
            && e.frames.len() <= 360_001
            && ((e.frames.len() - 1) as f64 * 0.01 - duration).abs() <= 0.0100625
            && e.frames
                .iter()
                .enumerate()
                .all(|(i, f)| f.time_seconds.is_finite()
                    && (f.time_seconds - i as f64 * 0.01).abs() < 1e-9
                    && f.time_seconds <= duration + 1. / RATE as f64
                    && f.confidence.is_finite()
                    && (0.0..=1.).contains(&f.confidence)
                    && f.rms.is_finite()
                    && f.rms >= 0.
                    && f.frequency_hz
                        .is_none_or(|hz| hz.is_finite() && (20. ..=2200.).contains(&hz))),
        "Invalid RMVPE contour"
    );
    Ok(())
}
