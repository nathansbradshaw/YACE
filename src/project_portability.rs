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
    if let Some(basis) = &doc.recording {
        validate_recording(files, basis, &doc.source_chart_sha256)?;
    }
    if !doc.has_recording() {
        ensure!(
            doc.audio_filename.is_empty() && doc.audio_sha256.is_empty(),
            "Chart-only project has a recording identity"
        );
        return Ok(());
    }
    ensure!(
        crate::stem_review::conventional::valid_audio_filename(&doc.audio_filename),
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
fn validate_recording<B: Bytes>(
    files: &BTreeMap<String, B>,
    basis: &crate::project_recording::Basis,
    source: &str,
) -> Result<()> {
    basis.validate(source)?;
    if let Some(recording) = &basis.recording {
        ensure!(
            files
                .get(&format!("source/{}", recording.filename))
                .is_some_and(|bytes| digest(bytes.bytes()) == recording.sha256),
            "Recording referenced by project history is missing or changed"
        );
    }
    Ok(())
}
fn key(audio: &str, manifest: &Option<StemManifest>) -> Result<String> {
    Ok(digest(&serde_json::to_vec(&(audio, manifest))?))
}
fn translated(
    hash: &str,
    index: &BTreeMap<String, Vec<String>>,
    transcodes: &BTreeMap<String, String>,
) -> Result<String> {
    let mut hash = hash.to_ascii_lowercase();
    let mut seen = BTreeSet::new();
    while !index.contains_key(&hash) {
        let Some(next) = transcodes.get(&hash) else {
            break;
        };
        ensure!(
            seen.insert(hash.clone()) && seen.len() <= 64,
            "Cyclic portable audio identity map"
        );
        hash = next.clone();
    }
    Ok(hash)
}
fn relocate_content(
    content: &mut Content,
    root: &Path,
    audio: &str,
    index: &BTreeMap<String, Vec<String>>,
    transcodes: &BTreeMap<String, String>,
    keys: &mut BTreeMap<String, String>,
) -> Result<()> {
    let old = key(audio, &content.stems)?;
    for manifest in content
        .stems
        .iter_mut()
        .chain(content.keyboard_stems.iter_mut())
    {
        let mut claimed = BTreeSet::new();
        let mut converted = false;
        for asset in &mut manifest.stems {
            let next = translated(&asset.sha256, index, transcodes)?;
            converted |= !next.eq_ignore_ascii_case(&asset.sha256);
            asset.sha256 = next;
            let candidates = index.get(&asset.sha256.to_ascii_lowercase());
            let available = || {
                candidates
                    .into_iter()
                    .flatten()
                    .filter(|name| !claimed.contains(name.as_str()))
            };
            let name = available()
                .filter(|name| asset.path.ends_with(Path::new(name.as_str())))
                .max_by_key(|name| name.len())
                .or_else(|| available().next());
            asset.path = if let Some(name) = name {
                claimed.insert(name.clone());
                root.join(name)
            } else {
                root.join("unavailable")
                    .join(format!("{}-{}.wav", asset.id.as_str(), asset.sha256))
            };
        }
        if converted {
            let detail = " Portable 24-bit listening copy; original audio identities are retained in yace-portable-audio.json.";
            if !manifest.alignment.details.contains(detail)
                && manifest.alignment.details.len() + detail.len() <= 10000
            {
                manifest.alignment.details.push_str(detail);
            }
            if let Some(identity) = &mut manifest.generation {
                if identity.parameters.len() < 100
                    || identity.parameters.contains_key("portable_audio")
                {
                    identity
                        .parameters
                        .insert("portable_audio".into(), "peak-scaled-pcm24-v1".into());
                    identity.cache_key =
                        identity.calculated_cache_key(&manifest.source_audio_sha256)?;
                }
            }
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
    let transcodes: BTreeMap<String, String> = files
        .get("yace-portable-audio.json")
        .map(|bytes| {
            ensure!(
                bytes.bytes().len() <= 16 * 1024 * 1024,
                "Portable audio map is too large"
            );
            Ok::<_, anyhow::Error>(serde_json::from_slice(bytes.bytes())?)
        })
        .transpose()?
        .unwrap_or_default();
    ensure!(
        transcodes.len() <= 16384
            && transcodes.iter().all(|(old, new)| [old, new]
                .iter()
                .all(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))),
        "Invalid portable audio identity map"
    );
    let mut index = BTreeMap::<String, Vec<String>>::new();
    let recording = format!("source/{}", doc.audio_filename);
    for (name, bytes) in files.iter() {
        ensure!(
            Path::new(name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Archive path leaves the project"
        );
        if name != &recording
            && Path::new(name)
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| {
                    ["wav", "flac", "ogg", "opus", "mp3", "m4a", "aiff", "aac"]
                        .contains(&s.to_ascii_lowercase().as_str())
                })
        {
            index
                .entry(digest(bytes.bytes()))
                .or_default()
                .push(name.clone());
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
        let mut checkpoint = crate::project_recording::Checkpoint::decode(bytes.bytes())?;
        if let Some(basis) = &checkpoint.recording {
            validate_recording(files, basis, &doc.source_chart_sha256)?;
        }
        let audio = checkpoint
            .recording
            .as_ref()
            .map_or(doc.audio_sha256.as_str(), |basis| basis.audio_hash());
        relocate_content(
            &mut checkpoint.content,
            root,
            audio,
            &index,
            &transcodes,
            &mut keys,
        )?;
        let bytes = checkpoint.encode()?;
        let next = digest(&bytes);
        files
            .entry(format!("checkpoints/{next}.json"))
            .or_insert(bytes.into());
        checkpoints.insert(id, next);
    }
    relocate_content(
        &mut doc.content,
        root,
        &doc.audio_sha256,
        &index,
        &transcodes,
        &mut keys,
    )?;
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
            let source = translated(&set.drum_audio_sha256, &index, &transcodes)?;
            if source != set.drum_audio_sha256 {
                let original = set.original_hash(&set.drum_audio_sha256).to_owned();
                set.portable_audio.insert(source.clone(), original);
                set.drum_audio_sha256 = source;
            }
            for part in &mut set.parts {
                let next = translated(&part.sha256, &index, &transcodes)?;
                if next != part.sha256 {
                    let original = set
                        .portable_audio
                        .get(&part.sha256)
                        .cloned()
                        .unwrap_or_else(|| part.sha256.clone());
                    set.portable_audio.insert(next.clone(), original);
                    part.sha256 = next;
                }
            }
            files.insert(name, serde_json::to_vec_pretty(&set)?.into());
        }
    }
    if let Some(selected) = &mut doc.kit_audition {
        selected.audio_sha256 = translated(&selected.audio_sha256, &index, &transcodes)?;
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
pub fn referenced_stems<B: Bytes>(
    files: &BTreeMap<String, B>,
) -> Result<BTreeMap<PathBuf, String>> {
    let doc: Project = serde_json::from_slice(
        files
            .get("project.json")
            .context("Missing project")?
            .bytes(),
    )?;
    let mut contents = vec![doc.content];
    for id in doc.undo.iter().chain(&doc.redo) {
        let bytes = files
            .get(&format!("checkpoints/{id}.json"))
            .context("Missing checkpoint")?;
        ensure!(digest(bytes.bytes()) == *id, "Checkpoint checksum differs");
        let checkpoint = crate::project_recording::Checkpoint::decode(bytes.bytes())?;
        if let Some(basis) = &checkpoint.recording {
            validate_recording(files, basis, &doc.source_chart_sha256)?;
        }
        contents.push(checkpoint.content);
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
