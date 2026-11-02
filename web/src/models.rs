//! Portable source identity and shared inference decoding.
#[path = "../../src/models/postprocessing.rs"]
pub mod postprocessing;
use anyhow::{Context, Result, ensure};
pub use postprocessing::{
    decode_pitch, decode_pitch_with_evidence, drum_peaks, pitch_frame_seconds, supported_drum_peaks,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelAsset {
    pub id: String,
    pub filename: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
}
pub fn assets() -> Vec<ModelAsset> {
    let mut assets: Vec<ModelAsset> =
        serde_json::from_str(include_str!("../../models/manifest.json")).expect("pinned manifest");
    assets.push(crate::lyric_language::multilingual_asset());
    assets.push(crate::keyboard_separation::asset());
    assets.push(crate::acoustic_alignment::asset());
    assets
}
pub fn hash_file(path: &Path) -> Result<String> {
    Ok(hash_shared(&crate::storage::shared(path)?))
}
pub fn hash_shared(bytes: &std::sync::Arc<Vec<u8>>) -> String {
    crate::audio_identity::hash(bytes)
}
/// Call only after hashing these bytes or receiving their verified worker output.
/// Weak ownership prevents allocator address reuse from confusing identities.
pub(crate) fn remember_verified_hash(bytes: &std::sync::Arc<Vec<u8>>, hash: &str) {
    crate::audio_identity::remember_verified(bytes, hash);
}
thread_local! { static PCM: std::cell::RefCell<std::collections::BTreeMap<PathBuf,(String,crate::audio_decode::Pcm)>> = std::cell::RefCell::default(); }
thread_local! { static STEMS: std::cell::RefCell<std::collections::BTreeSet<String>> = std::cell::RefCell::default(); }
pub(crate) fn decoded_bytes() -> usize {
    PCM.with(|cache| {
        cache
            .borrow()
            .values()
            .map(|(_, pcm)| pcm.stereo.len() * 4)
            .sum()
    })
}
fn stem_key(manifest: &crate::stem_review::StemManifest) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest)?)
    ))
}
pub(crate) fn remember_stems(manifest: &crate::stem_review::StemManifest) -> Result<()> {
    let key = stem_key(manifest)?;
    STEMS.with(|cache| cache.borrow_mut().insert(key));
    Ok(())
}
pub fn validate_stems(manifest: &crate::stem_review::StemManifest, source: &Path) -> Result<()> {
    ensure!(
        STEMS
            .with(|cache| Ok::<_, anyhow::Error>(cache.borrow().contains(&stem_key(manifest)?)))?,
        "Validate supplied stems on a browser worker first"
    );
    ensure!(
        hash_file(source)?.eq_ignore_ascii_case(&manifest.source_audio_sha256),
        "Source audio changed after stem validation"
    );
    for asset in &manifest.stems {
        ensure!(
            hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
            "Stem audio changed after validation"
        );
    }
    Ok(())
}
pub fn remember(path: &Path, pcm: crate::audio_decode::Pcm) -> Result<()> {
    let hash = hash_file(path)?;
    PCM.with(|cache| cache.borrow_mut().insert(path.to_owned(), (hash, pcm)));
    Ok(())
}
pub fn format(path: &Path) -> Result<(u32, usize)> {
    let hash = hash_file(path)?;
    PCM.with(|cache| {
        let cache = cache.borrow();
        let (bound, pcm) = cache
            .get(path)
            .context("Prepare audio on a browser worker first")?;
        ensure!(
            *bound == hash,
            "Decoded audio belongs to different source bytes"
        );
        Ok((pcm.rate, pcm.channels))
    })
}
pub fn duration(path: &Path) -> Result<f64> {
    let hash = hash_file(path)?;
    PCM.with(|cache| {
        let cache = cache.borrow();
        let (bound, pcm) = cache
            .get(path)
            .context("Prepare audio on a browser worker first")?;
        ensure!(
            *bound == hash,
            "Decoded audio belongs to different source bytes"
        );
        Ok(pcm.duration())
    })
}
pub fn decode(path: &Path, rate: u32, channels: u32) -> Result<Vec<f32>> {
    let hash = hash_file(path)?;
    PCM.with(|cache| {
        let cache = cache.borrow();
        let (bound, pcm) = cache
            .get(path)
            .context("Prepare audio on a browser worker first")?;
        ensure!(
            *bound == hash,
            "Decoded audio belongs to different source bytes"
        );
        pcm.samples(rate, channels)
    })
}
pub fn write_audio(path: &Path, left: &[f32], right: &[f32]) -> Result<()> {
    ensure!(left.len() == right.len(), "Unequal stereo channels");
    let pcm = crate::audio_decode::Pcm {
        stereo: left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect(),
        rate: 44100,
        channels: 2,
    };
    crate::storage::write_new(path, &pcm.float_wave()?)?;
    Ok(())
}
pub fn verified(_root: &Path, _id: &str) -> Result<PathBuf> {
    anyhow::bail!("This inference backend cannot run in the browser")
}
