//! Portable source identity; native inference is not linked into the browser.
use anyhow::Result;
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
    serde_json::from_str(include_str!("../../models/manifest.json")).expect("pinned manifest")
}
pub fn hash_file(path: &Path) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(crate::storage::shared(path)?.as_slice())
    ))
}
pub fn decode(_path: &Path, _rate: u32, _channels: usize) -> Result<Vec<f32>> {
    anyhow::bail!("Decode audio with the browser audio API")
}
pub fn write_audio(_path: &Path, _left: &[f32], _right: &[f32]) -> Result<()> {
    anyhow::bail!("Native audio rendering is unavailable in the static browser app")
}
pub fn verified(_root: &Path, _id: &str) -> Result<PathBuf> {
    anyhow::bail!("This inference backend cannot run in the browser")
}
