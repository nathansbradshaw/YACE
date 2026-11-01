//! Relocate a project copy without changing source bytes or authored content.
//! Checkpoints are content addressed: relocated copies get new IDs, while the
//! original checkpoint bytes remain available for recovery.
use crate::{
    editor::{Content, Project},
    stem_review::StemManifest,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub type Files = BTreeMap<String, Vec<u8>>;
pub trait Bytes: From<Vec<u8>> {
    fn bytes(&self) -> &[u8];
}
impl Bytes for Vec<u8> {
    fn bytes(&self) -> &[u8] {
        self
    }
}
impl Bytes for std::sync::Arc<Vec<u8>> {
    fn bytes(&self) -> &[u8] {
        self.as_slice()
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn validate_sources<B: Bytes>(files: &BTreeMap<String, B>, doc: &Project) -> Result<()> {
    ensure!(
        files
            .get("source/notes.chart")
            .is_some_and(|bytes| digest(bytes.bytes()) == doc.source_chart_sha256),
        "Project source chart checksum differs"
    );
    ensure!(
        Path::new(&doc.audio_filename).components().count() == 1,
        "Invalid project recording filename"
    );
    ensure!(
        files
            .get(&format!("source/{}", doc.audio_filename))
            .is_some_and(|bytes| digest(bytes.bytes()) == doc.audio_sha256),
        "Project source recording checksum differs"
    );
    Ok(())
}
fn key(audio: &str, manifest: &Option<StemManifest>) -> Result<String> {
    Ok(digest(&serde_json::to_vec(&(audio, manifest))?))
}
fn relocate_content(
    content: &mut Content,
    root: &Path,
    audio: &str,
    index: &BTreeMap<String, String>,
    keys: &mut BTreeMap<String, String>,
) -> Result<()> {
    let old = key(audio, &content.stems)?;
    for manifest in content
        .stems
        .iter_mut()
        .chain(content.keyboard_stems.iter_mut())
    {
        for asset in &mut manifest.stems {
            asset.path = if let Some(name) = index.get(&asset.sha256.to_ascii_lowercase()) {
                root.join(name)
            } else {
                root.join("unavailable")
                    .join(format!("{}.wav", asset.sha256))
            };
        }
    }
    keys.insert(old, key(audio, &content.stems)?);
    Ok(())
}
pub fn relocate<B: Bytes>(files: &mut BTreeMap<String, B>, root: &Path) -> Result<()> {
    ensure!(
        crate::storage::is_absolute(root),
        "A portable project needs an absolute destination"
    );
    let mut doc: Project = serde_json::from_slice(
        files
            .get("project.json")
            .context("Project ZIP has no project.json")?
            .bytes(),
    )?;
    validate_sources(files, &doc)?;
    let mut index = BTreeMap::new();
    for (name, bytes) in files.iter() {
        ensure!(
            Path::new(name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Archive path leaves the project"
        );
        if Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| {
                ["wav", "flac", "ogg", "opus", "mp3", "m4a"]
                    .contains(&s.to_ascii_lowercase().as_str())
            })
        {
            index
                .entry(digest(bytes.bytes()))
                .or_insert_with(|| name.clone());
        }
    }
    let mut keys = BTreeMap::new();
    let mut checkpoints = BTreeMap::new();
    let ids: BTreeSet<_> = doc.undo.iter().chain(&doc.redo).cloned().collect();
    for id in ids {
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid checkpoint ID"
        );
        let bytes = files
            .get(&format!("checkpoints/{id}.json"))
            .context("Project checkpoint is missing")?;
        ensure!(
            digest(bytes.bytes()) == id,
            "Project checkpoint checksum differs"
        );
        let mut content: Content = serde_json::from_slice(bytes.bytes())?;
        relocate_content(&mut content, root, &doc.audio_sha256, &index, &mut keys)?;
        let bytes = serde_json::to_vec(&content)?;
        let next = digest(&bytes);
        files
            .entry(format!("checkpoints/{next}.json"))
            .or_insert(bytes.into());
        checkpoints.insert(id, next);
    }
    relocate_content(&mut doc.content, root, &doc.audio_sha256, &index, &mut keys)?;
    for id in doc.undo.iter_mut().chain(doc.redo.iter_mut()) {
        *id = checkpoints
            .get(id)
            .context("Missing relocated checkpoint")?
            .clone();
    }
    let kits: Vec<_> = files
        .keys()
        .filter(|name| name.starts_with("analysis/kit-") && name.ends_with("/kit-parts.json"))
        .cloned()
        .collect();
    for name in kits {
        let Ok(mut set) =
            serde_json::from_slice::<crate::part_analysis::DrumPartSet>(files[&name].bytes())
        else {
            continue;
        };
        if let Some(next) = keys.get(&set.source_key)
            && set.original_audio_sha256 == doc.audio_sha256
        {
            set.source_key = next.clone();
            set.directory = root
                .join(Path::new(&name).parent().context("Invalid kit folder")?)
                .join("parts");
            files.insert(name, serde_json::to_vec_pretty(&set)?.into());
        }
    }
    if let Some(selected) = &mut doc.kit_audition {
        if let Some(next) = keys.get(&selected.source_key) {
            selected.source_key = next.clone();
        }
        let normalized = selected.kit.to_string_lossy().replace('\\', "/");
        if let Some(folder) = Path::new(&normalized).parent().and_then(Path::file_name) {
            selected.kit = root.join("analysis").join(folder).join("kit-parts.json");
        }
    }
    doc.transport.playing = false;
    files.insert("project.json".into(), serde_json::to_vec(&doc)?.into());
    Ok(())
}

/// Assets referenced by current and undo/redo snapshots are eligible for a
/// portable backup. Missing external assets remain explicitly unavailable.
pub fn referenced_stems(files: &Files) -> Result<BTreeMap<PathBuf, String>> {
    let doc: Project =
        serde_json::from_slice(files.get("project.json").context("Missing project")?)?;
    let mut contents = vec![doc.content];
    for id in doc.undo.iter().chain(&doc.redo) {
        let bytes = files
            .get(&format!("checkpoints/{id}.json"))
            .context("Missing checkpoint")?;
        ensure!(digest(bytes) == *id, "Checkpoint checksum differs");
        contents.push(serde_json::from_slice(bytes)?);
    }
    let mut out = BTreeMap::new();
    for content in contents {
        for manifest in content.stems.into_iter().chain(content.keyboard_stems) {
            for asset in manifest.stems {
                out.insert(asset.path, asset.sha256);
            }
        }
    }
    Ok(out)
}
