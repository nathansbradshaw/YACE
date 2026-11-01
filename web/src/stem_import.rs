//! Resolve imported stem bundles by content. Desktop absolute paths never grant
//! access to a browser file, and names alone cannot bind another recording.
use crate::{editor::Project, stem_review::StemManifest};
use anyhow::{Context, Result, ensure};
use std::path::Path;
pub fn manifest(
    files: &crate::files::Files,
    project: &Project,
    output: &Path,
) -> Result<Option<StemManifest>> {
    let manifests: Vec<_> = files
        .iter()
        .filter(|(name, bytes)| {
            name.to_ascii_lowercase().ends_with(".json") && bytes.len() <= 256 * 1024
        })
        .filter_map(|(_, bytes)| serde_json::from_slice::<StemManifest>(bytes).ok())
        .collect();
    ensure!(
        manifests.len() <= 1,
        "Choose a folder containing one stem manifest and its audio files"
    );
    let Some(mut manifest) = manifests.into_iter().next() else {
        return Ok(None);
    };
    let hashes: Vec<_> = files
        .iter()
        .map(|(_, bytes)| crate::models::hash_shared(bytes))
        .collect();
    for asset in &mut manifest.stems {
        let index = hashes
            .iter()
            .position(|hash| hash.eq_ignore_ascii_case(&asset.sha256))
            .with_context(|| {
                format!(
                    "The selected folder is missing the bound {} audio",
                    asset.id.as_str()
                )
            })?;
        let extension = Path::new(&files[index].0)
            .extension()
            .unwrap_or_default()
            .to_string_lossy();
        asset.path = output.join(format!("{}.{}", asset.id.as_str(), extension));
    }
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    for asset in &manifest.stems {
        let index = hashes
            .iter()
            .position(|hash| hash.eq_ignore_ascii_case(&asset.sha256))
            .unwrap();
        crate::storage::write_shared(&asset.path, files[index].1.clone())?;
    }
    Ok(Some(manifest))
}
