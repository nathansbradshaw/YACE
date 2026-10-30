//! Build piano-analysis audio from the original minus drums, bass and vocals.
//! Stem labels can leak other instruments, so keep the remaining harmonic audio
//! at its original gain and timing. Chart notes are not inputs to this step.
use crate::{
    models,
    stem_review::{StemId, StemManifest},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
pub const SAMPLE_RATE: u32 = 16000;
pub const RECIPE: &str = "harmonic-residual-rms-v1;original-minus-drums-bass-vocals;mono16000;whole-source-rms0.1;maxgain4;peakceiling0.95;scalar-only;no-limiter-no-trim-no-shift";
pub const TARGET_RMS: f64 = 0.1;
pub const MAX_GAIN: f64 = 4.0;
pub const PEAK_CEILING: f64 = 0.95;
pub const CONTRIBUTORS: [StemId; 3] = [StemId::Drums, StemId::Bass, StemId::Vocals];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub recipe: String,
    pub source_audio_sha256: String,
    pub contributing_stem_sha256: BTreeMap<String, String>,
    pub source_sample_rate: u32,
    pub source_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_count: u64,
    pub target_rms: f64,
    pub maximum_gain: f64,
    pub peak_ceiling: f64,
    pub gain: f64,
    pub residual_rms: f64,
    pub residual_peak: f64,
    pub analysis_rms: f64,
    pub analysis_peak: f64,
    pub residual_pcm_sha256: String,
    pub analysis_pcm_sha256: String,
}
pub struct Prepared {
    pub samples: Vec<f32>,
    pub receipt: Receipt,
}
pub struct Mixed {
    pub samples: Vec<f32>,
    pub gain: f64,
    pub residual_rms: f64,
    pub residual_peak: f64,
    pub analysis_rms: f64,
    pub analysis_peak: f64,
    pub residual_pcm_sha256: String,
    pub analysis_pcm_sha256: String,
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * (1. + a.abs().max(b.abs()))
}
pub fn pcm_hash(samples: &[f32]) -> String {
    let mut h = Sha256::new();
    for s in samples {
        h.update(s.to_le_bytes());
    }
    format!("{:x}", h.finalize())
}
fn stats(samples: &[f32]) -> (f64, f64) {
    let energy = samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>();
    let peak = samples.iter().map(|s| (*s as f64).abs()).fold(0., f64::max);
    // Summation roundoff can put constant-signal RMS a few ulps above peak.
    ((energy / samples.len() as f64).sqrt().min(peak), peak)
}
pub fn gain_for(rms: f64, peak: f64) -> Result<f64> {
    ensure!(
        rms.is_finite() && peak.is_finite() && rms >= 0. && peak >= rms,
        "Invalid residual RMS/peak"
    );
    Ok(if peak == 0. {
        1.
    } else {
        (TARGET_RMS / rms.max(f64::MIN_POSITIVE))
            .min(MAX_GAIN)
            .min(PEAK_CEILING / peak)
    })
}
/// Inputs must already share the source's mono16kHz sample grid. Subtraction is
/// performed in f64 then rounded once to f32; gain is one scalar for the entire
/// recording. Every original analysis sample is retained, including silence.
pub fn mix_aligned(original: &[f32], drums: &[f32], bass: &[f32], vocals: &[f32]) -> Result<Mixed> {
    ensure!(!original.is_empty(), "Empty piano analysis input");
    ensure!(
        [drums, bass, vocals]
            .iter()
            .all(|s| s.len() == original.len()),
        "Piano contributors must have exactly the source sample count"
    );
    ensure!(
        original
            .iter()
            .chain(drums)
            .chain(bass)
            .chain(vocals)
            .all(|s| s.is_finite()),
        "Nonfinite piano input"
    );
    let residual: Vec<f32> = original
        .iter()
        .zip(drums)
        .zip(bass)
        .zip(vocals)
        .map(|(((o, d), b), v)| (*o as f64 - *d as f64 - *b as f64 - *v as f64) as f32)
        .collect();
    ensure!(
        residual.iter().all(|s| s.is_finite()),
        "Piano residual overflow"
    );
    let (residual_rms, residual_peak) = stats(&residual);
    let gain = gain_for(residual_rms, residual_peak)?;
    let samples: Vec<f32> = residual.iter().map(|s| (*s as f64 * gain) as f32).collect();
    let (analysis_rms, analysis_peak) = stats(&samples);
    ensure!(
        analysis_peak <= PEAK_CEILING + 1e-7,
        "Piano analysis peak bound exceeded"
    );
    Ok(Mixed {
        residual_pcm_sha256: pcm_hash(&residual),
        analysis_pcm_sha256: pcm_hash(&samples),
        samples,
        gain,
        residual_rms,
        residual_peak,
        analysis_rms,
        analysis_peak,
    })
}
impl Receipt {
    /// Validate a saved recipe offline: actual source/stem identities are bound to
    /// the saved manifest, without requiring any audio or models to be installed.
    pub fn validate(&self, manifest: &StemManifest) -> Result<()> {
        manifest.validate_binding(
            &self.source_audio_sha256,
            self.source_frames as f64 / 44100.,
        )?;
        ensure!(
            self.version == 1 && self.recipe == RECIPE,
            "Unsupported piano audio recipe"
        );
        ensure!(
            self.source_sample_rate == 44100
                && self.source_frames == manifest.frames
                && self.sample_rate == SAMPLE_RATE
                && self.channels == 1
                && self.sample_count > 0,
            "Piano analysis source/sample grid differs"
        );
        ensure!(
            (self.sample_count as f64 / SAMPLE_RATE as f64 - self.source_frames as f64 / 44100.)
                .abs()
                <= 1. / SAMPLE_RATE as f64,
            "Piano analysis must cover the whole source"
        );
        let expected = contributing_hashes(manifest)?;
        ensure!(
            self.contributing_stem_sha256 == expected,
            "Piano analysis contributor hashes differ from bound stems"
        );
        ensure!(
            [
                &self.source_audio_sha256,
                &self.residual_pcm_sha256,
                &self.analysis_pcm_sha256
            ]
            .iter()
            .all(|h| valid_hash(h))
                && self
                    .contributing_stem_sha256
                    .values()
                    .all(|h| valid_hash(h)),
            "Invalid piano analysis SHA256"
        );
        ensure!(
            self.target_rms == TARGET_RMS
                && self.maximum_gain == MAX_GAIN
                && self.peak_ceiling == PEAK_CEILING,
            "Piano analysis gain policy changed"
        );
        ensure!(
            [
                self.gain,
                self.residual_rms,
                self.residual_peak,
                self.analysis_rms,
                self.analysis_peak
            ]
            .iter()
            .all(|s| s.is_finite() && *s >= 0.),
            "Invalid piano analysis levels"
        );
        ensure!(
            close(self.gain, gain_for(self.residual_rms, self.residual_peak)?)
                && self.gain > 0.
                && self.gain <= MAX_GAIN,
            "Piano analysis gain differs from frozen policy"
        );
        ensure!(
            close(self.analysis_rms, self.residual_rms * self.gain)
                && close(self.analysis_peak, self.residual_peak * self.gain)
                && self.analysis_peak <= PEAK_CEILING + 1e-7
                && self.analysis_peak >= self.analysis_rms,
            "Piano analysis levels differ from scalar gain"
        );
        Ok(())
    }
}
pub fn contributing_hashes(manifest: &StemManifest) -> Result<BTreeMap<String, String>> {
    CONTRIBUTORS
        .iter()
        .map(|id| {
            let s = manifest
                .stems
                .iter()
                .find(|s| s.id == *id)
                .with_context(|| {
                    format!(
                        "Piano harmonic residual requires bound {} stem",
                        id.as_str()
                    )
                })?;
            Ok((id.as_str().into(), s.sha256.clone()))
        })
        .collect()
}
/// Decode only the actual contributors. Manifest loading checks native stereo
/// format, every source-frame count and hashes before/after decoding. NN input
/// conversion then uses the same FFmpeg mono16kHz path as Kong itself.
pub fn prepare(source: &Path, manifest: &StemManifest) -> Result<Prepared> {
    let hashes = contributing_hashes(manifest)?;
    let mut subset = manifest.clone();
    subset.stems.retain(|s| CONTRIBUTORS.contains(&s.id));
    drop(subset.load(source)?);
    let original = models::decode(source, SAMPLE_RATE, 1)?;
    let mut pcm = BTreeMap::new();
    for id in CONTRIBUTORS {
        let asset = subset.stems.iter().find(|s| s.id == id).unwrap();
        pcm.insert(id, models::decode(&asset.path, SAMPLE_RATE, 1)?);
    }
    let mixed = mix_aligned(
        &original,
        &pcm[&StemId::Drums],
        &pcm[&StemId::Bass],
        &pcm[&StemId::Vocals],
    )?;
    ensure!(
        models::hash_file(source)? == manifest.source_audio_sha256,
        "Piano source changed during analysis input preparation"
    );
    for asset in &subset.stems {
        ensure!(
            models::hash_file(&asset.path)? == asset.sha256,
            "Piano contributor changed during analysis input preparation"
        );
    }
    let receipt = Receipt {
        version: 1,
        recipe: RECIPE.into(),
        source_audio_sha256: manifest.source_audio_sha256.clone(),
        contributing_stem_sha256: hashes,
        source_sample_rate: 44100,
        source_frames: manifest.frames,
        sample_rate: SAMPLE_RATE,
        channels: 1,
        sample_count: mixed.samples.len() as u64,
        target_rms: TARGET_RMS,
        maximum_gain: MAX_GAIN,
        peak_ceiling: PEAK_CEILING,
        gain: mixed.gain,
        residual_rms: mixed.residual_rms,
        residual_peak: mixed.residual_peak,
        analysis_rms: mixed.analysis_rms,
        analysis_peak: mixed.analysis_peak,
        residual_pcm_sha256: mixed.residual_pcm_sha256,
        analysis_pcm_sha256: mixed.analysis_pcm_sha256,
    };
    receipt.validate(manifest)?;
    Ok(Prepared {
        samples: mixed.samples,
        receipt,
    })
}
